//! Authenticated v11 codec negotiation, including hostile handshake inputs.
use super::*;
use intralink_core::{
    capabilities::CodecCapabilities,
    media::{MAX_DELTA_BYTES, apply_keyframe, encode_keyframe},
};
use std::future::ready;

const PROTOCOL: ViewProtocol = ViewProtocol::Negotiated;

async fn endpoints() -> Result<(Endpoint, Endpoint, SocketAddr)> {
    let host =
        bind_view_with_protocol(SecretKey::generate(), "127.0.0.1:0".parse()?, PROTOCOL).await?;
    let viewer = crate::bind(SecretKey::generate(), "127.0.0.1:0".parse()?, false).await?;
    let address = host
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("Missing loopback")?;
    Ok((host, viewer, address))
}

fn ceiling(codec: CodecMode) -> VideoConfig {
    VideoConfig::new(1080, 60, 75, 30).unwrap().with_mode(codec)
}

#[tokio::test]
async fn chooses_mutual_compact_and_reconstructs_real_pixels() -> Result<()> {
    let (host, viewer, addr) = endpoints().await?;
    let config = ceiling(CodecMode::Compact);
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        let req = timeout(IO_LIMIT, requests.recv())
            .await?
            .context("Missing capture")?;
        assert_eq!(req.desktop_config, Some(config));
        let rgb = [42, 17, 91].repeat(1920 * 1080);
        let mut data = req.buffer;
        ensure!(
            encode_keyframe(config, &rgb, &mut data, MAX_DELTA_BYTES)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?,
            "Keyframe budget"
        );
        ensure!(
            req.reply
                .send(Ok(Capture::AdaptiveFrame { height: 1080, data }))
                .is_ok(),
            "Capture canceled"
        );
        let req = timeout(IO_LIMIT, requests.recv())
            .await?
            .context("Missing end request")?;
        assert_eq!(req.desktop_config, Some(config));
        ensure!(req.reply.send(Ok(Capture::End)).is_ok(), "End canceled");
        Ok::<(), anyhow::Error>(())
    };
    let mut count = 0;
    let receive = view_with_observations(
        &viewer,
        host.id(),
        std::slice::from_ref(&addr),
        ViewMode {
            codec: CodecMode::Compact,
            protocol: PROTOCOL,
        },
        Vec::new,
        |frame| {
            let result = (|| -> Result<()> {
                assert_eq!(frame.config, config);
                assert_eq!(frame.sequence, 1);
                let mut pixels = Vec::new();
                apply_keyframe(config, &frame.payload, &mut pixels)
                    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
                assert_eq!(pixels.len(), 1920 * 1080);
                assert!(pixels.iter().all(|&p| p == 0x2a115b));
                count += 1;
                Ok(())
            })();
            ready(result)
        },
        |_| ViewPriority::Foreground,
    );
    let (sent, received, ()) = tokio::try_join!(
        serve_view_with_protocol(&host, viewer.id(), config, source, PROTOCOL),
        receive,
        producer
    )?;
    assert_eq!(sent, received);
    assert_eq!(count, 1);
    viewer.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn different_codec_ceilings_negotiate_before_capture() -> Result<()> {
    for (host_mode, viewer_mode, expected) in [
        (CodecMode::Compact, CodecMode::Tiles, CodecMode::Tiles),
        (CodecMode::Jpeg, CodecMode::Compact, CodecMode::Jpeg),
        (CodecMode::Compact, CodecMode::Jpeg, CodecMode::Jpeg),
    ] {
        let (host, viewer, addr) = endpoints().await?;
        let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
        let producer = async {
            let req = timeout(IO_LIMIT, requests.recv())
                .await?
                .context("Missing capture")?;
            assert_eq!(req.desktop_config, Some(ceiling(expected)));
            ensure!(req.reply.send(Ok(Capture::End)).is_ok(), "End canceled");
            Ok::<(), anyhow::Error>(())
        };
        let (sent, viewed, ()) = tokio::try_join!(
            serve_view_with_protocol(&host, viewer.id(), ceiling(host_mode), source, PROTOCOL),
            view_with_observations(
                &viewer,
                host.id(),
                std::slice::from_ref(&addr),
                ViewMode {
                    codec: viewer_mode,
                    protocol: PROTOCOL
                },
                Vec::new,
                |_| -> std::future::Ready<Result<()>> { panic!("Unexpected media") },
                |_| ViewPriority::Foreground
            ),
            producer
        )?;
        assert_eq!(sent, viewed);
        assert_eq!(viewed.frames, 0);
        viewer.close().await;
        host.close().await;
    }
    Ok(())
}

#[tokio::test]
async fn invalid_offer_or_confirmation_cannot_start_capture() -> Result<()> {
    for mask in [0, 8, 4, 1] {
        let (host, viewer, addr) = endpoints().await?;
        let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
        let peer = async {
            let conn = viewer
                .connect(
                    EndpointAddr::new(host.id()).with_ip_addr(addr),
                    PROTOCOL.alpn(),
                )
                .await?;
            let (mut tx, mut rx) = conn.open_bi().await?;
            let mut offer = PROTOCOL.request(CodecMode::Jpeg);
            offer[16] = mask;
            tx.write_all(&offer).await?;
            if mask == 1 {
                let mut selection = [0; CONFIG_BYTES + 1];
                timeout(IO_LIMIT, rx.read_exact(&mut selection)).await??;
                selection[0] ^= 1;
                tx.write_all(&selection).await?;
            }
            let mut extra = [0; 1];
            assert!(timeout(IO_LIMIT, rx.read_exact(&mut extra)).await?.is_err());
            Ok::<(), anyhow::Error>(())
        };
        let (sent, received) = tokio::join!(
            serve_view_with_protocol(
                &host,
                viewer.id(),
                ceiling(CodecMode::Jpeg),
                source,
                PROTOCOL
            ),
            peer
        );
        received?;
        assert!(sent.is_err());
        assert!(requests.recv().await.is_none());
        viewer.close().await;
        host.close().await;
    }
    Ok(())
}

#[tokio::test]
async fn viewer_rejects_host_selection_outside_intersection() -> Result<()> {
    let (host, viewer, addr) = endpoints().await?;
    let peer = async {
        let incoming = timeout(IO_LIMIT, host.accept())
            .await?
            .context("Missing connection")?;
        let conn = incoming.await?;
        let (mut tx, mut rx) = conn.accept_bi().await?;
        let mut request = [0; REQUEST_BYTES];
        rx.read_exact(&mut request).await?;
        assert_eq!(
            CodecCapabilities::parse_request(&request).unwrap().bits(),
            7
        );
        let mut selection = [0; CONFIG_BYTES + 1];
        selection[0] = 7;
        // Both support Compact; silently choosing JPEG is not authorized.
        selection[1..].copy_from_slice(&ceiling(CodecMode::Jpeg).encode());
        tx.write_all(&selection).await?;
        let mut confirmation = [0; CONFIG_BYTES + 1];
        assert!(
            timeout(IO_LIMIT, rx.read_exact(&mut confirmation))
                .await?
                .is_err()
        );
        Ok::<(), anyhow::Error>(())
    };
    let (sent, viewed) = tokio::join!(
        peer,
        view_with_observations(
            &viewer,
            host.id(),
            std::slice::from_ref(&addr),
            ViewMode {
                codec: CodecMode::Compact,
                protocol: PROTOCOL
            },
            Vec::new,
            |_| -> std::future::Ready<Result<()>> { panic!("Invalid selection reached consumer") },
            |_| ViewPriority::Foreground
        )
    );
    sent?;
    assert!(viewed.is_err());
    viewer.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn negotiation_requires_identity_and_explicit_version() -> Result<()> {
    let (host, viewer, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let (served, viewed) = tokio::join!(
        serve_view_with_protocol(
            &host,
            SecretKey::generate().public(),
            ceiling(CodecMode::Compact),
            source,
            PROTOCOL
        ),
        view_with_observations(
            &viewer,
            host.id(),
            std::slice::from_ref(&addr),
            ViewMode {
                codec: CodecMode::Compact,
                protocol: PROTOCOL
            },
            Vec::new,
            |_| -> std::future::Ready<Result<()>> { panic!("Unauthorized media") },
            |_| ViewPriority::Foreground
        )
    );
    assert!(served.is_err() && viewed.is_err());
    assert!(requests.recv().await.is_none());
    viewer.close().await;
    host.close().await;

    let (host, viewer, addr) = endpoints().await?;
    let client = timeout(
        IO_LIMIT,
        viewer.connect(
            EndpointAddr::new(host.id()).with_ip_addr(addr),
            ViewProtocol::Adaptive.alpn(),
        ),
    );
    let reject = async {
        if let Ok(Some(incoming)) = timeout(Duration::from_secs(2), host.accept()).await {
            assert!(incoming.await.is_err());
        }
    };
    let (connected, ()) = tokio::join!(client, reject);
    assert!(!matches!(connected, Ok(Ok(_))));
    viewer.close().await;
    host.close().await;
    Ok(())
}
