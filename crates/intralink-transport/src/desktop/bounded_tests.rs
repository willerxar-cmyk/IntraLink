//! Authenticated v12 profile negotiation and malformed-handshake regressions.
use super::*;
use intralink_core::{
    capabilities::CodecCapabilities,
    profile_limits::{self, ReceiverLimits},
};
use std::future::ready;

const PROTOCOL: ViewProtocol = ViewProtocol::Bounded;

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

fn ceiling() -> VideoConfig {
    VideoConfig::new(1080, 60, 75, 30)
        .unwrap()
        .with_mode(CodecMode::Compact)
}

#[tokio::test]
async fn bounded_selection_reduces_codec_height_and_fps_before_capture() -> Result<()> {
    let (host, viewer, addr) = endpoints().await?;
    let limits = ReceiverLimits::new(720, 24).unwrap();
    let mode = ViewMode::new(CodecMode::Jpeg, PROTOCOL).with_limits(limits);
    let expected = VideoConfig::new(720, 24, 75, 30)
        .unwrap()
        .with_mode(CodecMode::Jpeg);
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        let req = timeout(IO_LIMIT, requests.recv())
            .await?
            .context("Missing capture")?;
        assert_eq!(req.desktop_config, Some(expected));
        ensure!(req.reply.send(Ok(Capture::End)).is_ok(), "End canceled");
        Ok::<(), anyhow::Error>(())
    };
    let (sent, viewed, ()) = tokio::try_join!(
        serve_view_with_protocol(&host, viewer.id(), ceiling(), source, PROTOCOL),
        view_with_observations(
            &viewer,
            host.id(),
            std::slice::from_ref(&addr),
            mode,
            Vec::new,
            |_| -> std::future::Ready<Result<()>> { panic!("Unexpected media") },
            |_| ViewPriority::Foreground
        ),
        producer
    )?;
    assert_eq!(sent, viewed);
    viewer.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn malformed_limits_and_wrong_confirmation_never_request_capture() -> Result<()> {
    for limits in [[0, 0, 60], [4, 56, 61], [2, 208, 0], [2, 208, 30]] {
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
            tx.write_all(&PROTOCOL.request(CodecMode::Compact)).await?;
            tx.write_all(&limits).await?;
            if ReceiverLimits::decode(limits).is_some() {
                let mut selection = [0; profile_limits::SELECTION_BYTES];
                timeout(IO_LIMIT, rx.read_exact(&mut selection)).await??;
                selection[0] ^= 1;
                tx.write_all(&selection).await?;
            }
            let mut extra = [0; 1];
            assert!(timeout(IO_LIMIT, rx.read_exact(&mut extra)).await?.is_err());
            Ok::<(), anyhow::Error>(())
        };
        let (served, produced) = tokio::join!(
            serve_view_with_protocol(&host, viewer.id(), ceiling(), source, PROTOCOL),
            peer
        );
        produced?;
        assert!(served.is_err());
        assert!(requests.recv().await.is_none());
        viewer.close().await;
        host.close().await;
    }
    Ok(())
}

#[tokio::test]
async fn viewer_rejects_host_exceeding_receiver_limits_before_confirmation() -> Result<()> {
    let (host, viewer, addr) = endpoints().await?;
    let limits = ReceiverLimits::new(540, 15).unwrap();
    let mode = ViewMode::new(CodecMode::Compact, PROTOCOL).with_limits(limits);
    let peer = async {
        let incoming = timeout(IO_LIMIT, host.accept())
            .await?
            .context("Missing connection")?;
        let conn = incoming.await?;
        let (mut tx, mut rx) = conn.accept_bi().await?;
        let mut request = [0; REQUEST_BYTES];
        rx.read_exact(&mut request).await?;
        assert_eq!(
            profile_limits::parse_request(&request),
            Some(CodecCapabilities::through(mode.codec))
        );
        let mut bytes = [0; profile_limits::LIMIT_BYTES];
        rx.read_exact(&mut bytes).await?;
        assert_eq!(ReceiverLimits::decode(bytes), Some(limits));
        tx.write_all(&profile_limits::selection(ceiling(), ceiling()))
            .await?;
        let mut confirmation = [0; profile_limits::SELECTION_BYTES];
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
            mode,
            Vec::new,
            |_| ready(Err(anyhow::anyhow!("Invalid selection reached consumer"))),
            |_| ViewPriority::Foreground
        )
    );
    sent?;
    assert!(viewed.is_err());
    viewer.close().await;
    host.close().await;
    Ok(())
}
