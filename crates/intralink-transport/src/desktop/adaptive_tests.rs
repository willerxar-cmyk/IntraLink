//! Authenticated software v10 loopback tests, independent of native capture.
use super::*;
use intralink_core::{
    adaptive::{ViewProtocol, config_at_height},
    media::{MAX_DELTA_BYTES, TILE_TAG, apply_keyframe, encode_keyframe},
};
use std::future::ready;

const WAIT: Duration = Duration::from_secs(15);
const MODE: ViewMode = ViewMode::new(CodecMode::Compact, ViewProtocol::Adaptive);

fn ceiling() -> VideoConfig {
    VideoConfig::new(1080, 60, 75, 60)
        .unwrap()
        .with_mode(MODE.codec)
}

async fn endpoints() -> Result<(Endpoint, Endpoint, SocketAddr)> {
    let host =
        bind_view_with_protocol(SecretKey::generate(), "127.0.0.1:0".parse()?, MODE.protocol)
            .await?;
    let viewer = crate::bind(SecretKey::generate(), "127.0.0.1:0".parse()?, false).await?;
    let address = host
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("Missing loopback address")?;
    Ok((host, viewer, address))
}

#[tokio::test]
async fn adaptive_downshift_and_recovery_preserve_one_authenticated_session() -> Result<()> {
    let (host, viewer, address) = endpoints().await?;
    let approved = ceiling();
    let heights = [1080u16, 720, 540, 720, 1080];
    let mut payloads = Vec::new();
    for height in heights {
        let config = config_at_height(approved, height).unwrap();
        let rgb = [height as u8, 11, 22].repeat(config.width() as usize * config.height() as usize);
        let mut payload = Vec::new();
        ensure!(
            encode_keyframe(config, &rgb, &mut payload, MAX_DELTA_BYTES)
                .map_err(|err| anyhow::anyhow!("Keyframe: {err:?}"))?,
            "Fixture budget"
        );
        payloads.push(payload);
    }
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        for (index, height) in heights.into_iter().enumerate() {
            let count = if index == 4 { 1 } else { 16 };
            for _ in 0..count {
                let req = timeout(WAIT, requests.recv())
                    .await?
                    .context("Missing capture request")?;
                let mut data = req.buffer;
                data.clear();
                data.extend_from_slice(&payloads[index]);
                ensure!(
                    req.reply
                        .send(Ok(Capture::AdaptiveFrame { height, data }))
                        .is_ok(),
                    "Capture canceled"
                );
            }
        }
        let req = timeout(WAIT, requests.recv())
            .await?
            .context("Missing end request")?;
        ensure!(req.reply.send(Ok(Capture::End)).is_ok(), "End canceled");
        Ok::<(), anyhow::Error>(())
    };
    let mut sequence = 0usize;
    let mut canvas = Vec::new();
    let receiving = view_with_observations(
        &viewer,
        host.id(),
        std::slice::from_ref(&address),
        MODE,
        Vec::new,
        |frame| {
            let result = (|| -> Result<()> {
                let height = heights[(sequence / 16).min(4)];
                sequence += 1;
                let config = config_at_height(approved, height).unwrap();
                ensure!(
                    frame.sequence == sequence && frame.config == config,
                    "Unexpected adaptive profile"
                );
                apply_keyframe(config, &frame.payload, &mut canvas)
                    .map_err(|err| anyhow::anyhow!("Decode: {err:?}"))?;
                let pixel = (u32::from(height as u8) << 16) | (11 << 8) | 22;
                ensure!(
                    canvas.len() == config.width() as usize * config.height() as usize,
                    "Wrong canvas size"
                );
                ensure!(
                    canvas.first() == Some(&pixel) && canvas.last() == Some(&pixel),
                    "Wrong decoded pixels"
                );
                Ok(())
            })();
            ready(result)
        },
        |_| ViewPriority::Foreground,
    );
    let (sent, received, ()) = timeout(Duration::from_secs(70), async {
        tokio::try_join!(
            serve_view_with_protocol(&host, viewer.id(), approved, source, MODE.protocol),
            receiving,
            producer
        )
    })
    .await??;
    assert_eq!(sent, received);
    assert_eq!(received.frames, 65);
    assert_eq!(sequence, 65);
    host.close().await;
    viewer.close().await;
    Ok(())
}

#[tokio::test]
async fn adaptive_rejects_delta_as_first_frame_without_delivery() -> Result<()> {
    let (host, viewer, address) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        let request = timeout(WAIT, requests.recv())
            .await?
            .context("Missing capture request")?;
        ensure!(
            request
                .reply
                .send(Ok(Capture::AdaptiveFrame {
                    height: 1080,
                    data: vec![TILE_TAG, 1]
                }))
                .is_ok(),
            "Capture canceled"
        );
        Ok::<(), anyhow::Error>(())
    };
    let (sent, received, produced) = tokio::join!(
        serve_view_with_protocol(&host, viewer.id(), ceiling(), source, MODE.protocol),
        view_with_observations(
            &viewer,
            host.id(),
            std::slice::from_ref(&address),
            MODE,
            Vec::new,
            |_| -> std::future::Ready<Result<()>> { panic!("Malformed frame reached consumer") },
            |_| ViewPriority::Foreground
        ),
        producer,
    );
    produced?;
    assert!(sent.is_err());
    assert!(received.is_err());
    host.close().await;
    viewer.close().await;
    Ok(())
}

#[tokio::test]
async fn wrong_identity_cannot_request_adaptive_capture() -> Result<()> {
    let (host, viewer, address) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let (served, received) = tokio::join!(
        serve_view_with_protocol(
            &host,
            SecretKey::generate().public(),
            ceiling(),
            source,
            MODE.protocol
        ),
        view_with_observations(
            &viewer,
            host.id(),
            std::slice::from_ref(&address),
            MODE,
            Vec::new,
            |_| -> std::future::Ready<Result<()>> { panic!("Unauthorized frame") },
            |_| ViewPriority::Foreground
        ),
    );
    assert!(served.is_err() && received.is_err());
    assert!(requests.recv().await.is_none());
    host.close().await;
    viewer.close().await;
    Ok(())
}

#[tokio::test]
async fn invalid_peer_geometry_is_rejected_before_consumer_and_receipt() -> Result<()> {
    let (host, viewer, address) = endpoints().await?;
    let peer = async {
        let incoming = timeout(IO_LIMIT, host.accept())
            .await?
            .context("Missing connection")?;
        let connection = incoming.await?;
        let (mut tx, mut rx) = connection.accept_bi().await?;
        let mut request = [0; REQUEST_BYTES];
        rx.read_exact(&mut request).await?;
        assert_eq!(request, MODE.protocol.request(MODE.codec));
        tx.write_all(&ceiling().encode()).await?;
        let payload = [intralink_core::media::JPEG_TAG, 1];
        tx.write_all(&frame_header(payload.len()).unwrap()).await?;
        tx.write_all(&1440u16.to_be_bytes()).await?;
        tx.write_all(&payload).await?;
        let mut receipt = [0; 4];
        assert!(
            timeout(IO_LIMIT, rx.read_exact(&mut receipt))
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
            std::slice::from_ref(&address),
            MODE,
            Vec::new,
            |_| -> std::future::Ready<Result<()>> { panic!("Invalid geometry reached consumer") },
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
async fn v8_cannot_negotiate_adaptive_only_endpoint() -> Result<()> {
    let (host, viewer, address) = endpoints().await?;
    let client = timeout(
        IO_LIMIT,
        viewer.connect(EndpointAddr::new(host.id()).with_ip_addr(address), ALPN),
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
