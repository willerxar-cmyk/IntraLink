use super::*;
use intralink_core::h264::{self, Profile};
use std::future::ready;

// Real NVENC SPS/PPS with structural slice prefix; never sent to native decoder.
const IDR: &[u8] = &[
    0x00, 0x00, 0x01, 0x09, 0x10, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x20, 0xac, 0x2b, 0x20, 0x1e,
    0x02, 0x2f, 0xde, 0x02, 0xd4, 0x04, 0x04, 0x07, 0xc0, 0x00, 0x00, 0xfa, 0x00, 0x00, 0x75, 0x30,
    0x21, 0x00, 0x00, 0x01, 0x68, 0xeb, 0x8f, 0x2c, 0x00, 0x00, 0x01, 0x65, 0xb8,
];
const SLOW_IDR: &[u8] = &[
    0x00, 0x00, 0x01, 0x09, 0x10, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x20, 0xac, 0x2b, 0x20, 0x1e,
    0x02, 0x2f, 0xde, 0x02, 0xd4, 0x04, 0x04, 0x07, 0xc0, 0x00, 0x00, 0xfa, 0x00, 0x00, 0x03, 0x01,
    0xf4, 0x21, 0x00, 0x00, 0x01, 0x68, 0xeb, 0x8f, 0x2c, 0x00, 0x00, 0x01, 0x65, 0xb8,
];
const P: &[u8] = &[0, 0, 1, 9, 0x30, 0, 0, 1, 0x61, 0xe0];

fn profile() -> Profile {
    Profile::new(540, 60, 3, 8_000_000).unwrap()
}

async fn endpoints() -> Result<(Endpoint, Endpoint, SocketAddr)> {
    let host = bind_h264_diagnostic(SecretKey::generate(), "127.0.0.1:0".parse()?).await?;
    let viewer = crate::bind(SecretKey::generate(), "127.0.0.1:0".parse()?, false).await?;
    let addr = host
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("Missing loopback")?;
    Ok((host, viewer, addr))
}

fn frame(sequence: u32, keyframe: bool, data: &[u8], mut buffer: Vec<u8>) -> Result<Capture> {
    h264::wrap(sequence, keyframe, data, &mut buffer).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(Capture::Frame(buffer))
}

#[tokio::test]
async fn ordered_h264_records_keep_coded_sequence_distinct_from_receipts() -> Result<()> {
    let (host, viewer, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        for i in 0..6 {
            let req = requests.recv().await.context("Missing demand")?;
            let capture = match i {
                0 => frame(1, true, IDR, req.buffer)?,
                1 => Capture::NoUpdate(req.buffer),
                2 => frame(2, false, P, req.buffer)?,
                3 => Capture::Repeat(req.buffer),
                4 => frame(3, true, IDR, req.buffer)?,
                _ => Capture::End,
            };
            ensure!(req.reply.send(Ok(capture)).is_ok(), "Capture canceled");
        }
        Ok::<(), anyhow::Error>(())
    };
    let mut observed = Vec::new();
    let (served, viewed, produced) = tokio::join!(
        serve_h264_diagnostic(&host, viewer.id(), profile(), source),
        view_h264_diagnostic(
            &viewer,
            host.id(),
            std::slice::from_ref(&addr),
            profile(),
            |f| {
                let result = (|| {
                    ensure!(f.profile == profile(), "Wrong profile");
                    let au = h264::unwrap(&f.payload).map_err(|e| anyhow::anyhow!("{e:?}"))?;
                    observed.push((f.record_sequence, au.sequence, au.keyframe));
                    Ok(())
                })();
                ready(result)
            }
        ),
        producer
    );
    produced?;
    let served = served?;
    assert_eq!(served, viewed?);
    assert_eq!(
        (served.frames, served.repeats, served.no_updates),
        (3, 1, 1)
    );
    assert_eq!(observed, vec![(1, 1, true), (3, 2, false), (5, 3, true)]);
    viewer.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn unapproved_profile_never_requests_capture() -> Result<()> {
    let (host, viewer, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel(1);
    let incompatible = Profile::new(720, 60, 3, 8_000_000).unwrap();
    let (served, viewed) = tokio::join!(
        serve_h264_diagnostic(&host, viewer.id(), profile(), source),
        view_h264_diagnostic(
            &viewer,
            host.id(),
            std::slice::from_ref(&addr),
            incompatible,
            |_| ready(Ok(()))
        )
    );
    assert!(served.is_err() && viewed.is_err());
    assert!(requests.try_recv().is_err());
    viewer.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn unapproved_identity_never_requests_h264_capture() -> Result<()> {
    let (host, viewer, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel(1);
    let (served, viewed) = tokio::join!(
        serve_h264_diagnostic(&host, SecretKey::generate().public(), profile(), source),
        view_h264_diagnostic(
            &viewer,
            host.id(),
            std::slice::from_ref(&addr),
            profile(),
            |_| ready(Ok(()))
        )
    );
    assert!(served.is_err() && viewed.is_err());
    assert!(requests.try_recv().is_err());
    viewer.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn invalid_local_access_unit_never_reaches_consumer() -> Result<()> {
    let (host, viewer, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        let req = requests.recv().await.context("Missing demand")?;
        ensure!(
            req.reply.send(Ok(frame(1, false, P, req.buffer)?)).is_ok(),
            "Capture canceled"
        );
        Ok::<(), anyhow::Error>(())
    };
    let mut count = 0;
    let (served, viewed, produced) = tokio::join!(
        serve_h264_diagnostic(&host, viewer.id(), profile(), source),
        view_h264_diagnostic(
            &viewer,
            host.id(),
            std::slice::from_ref(&addr),
            profile(),
            |_| {
                count += 1;
                ready(Ok(()))
            }
        ),
        producer
    );
    produced?;
    assert!(served.is_err() && viewed.is_err());
    assert_eq!(count, 0);
    viewer.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn invalid_peer_access_unit_is_rejected_before_consumer_and_ack() -> Result<()> {
    let (host, viewer, addr) = endpoints().await?;
    let peer = async {
        let incoming = timeout(IO_LIMIT, host.accept())
            .await?
            .context("Missing connection")?;
        let connection = incoming.await?;
        let (mut tx, mut rx) = connection.accept_bi().await?;
        let mut req = [0; REQUEST_BYTES];
        rx.read_exact(&mut req).await?;
        assert_eq!(&req, h264::REQUEST);
        let mut offer = [0; h264::CONFIG_BYTES];
        rx.read_exact(&mut offer).await?;
        assert_eq!(Profile::decode(offer), Some(profile()));
        tx.write_all(&profile().encode()).await?;
        let mut payload = Vec::new();
        h264::wrap(1, false, P, &mut payload).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        tx.write_all(&frame_header(payload.len()).unwrap()).await?;
        tx.write_all(&payload).await?;
        let mut receipt = [0; 4];
        assert!(
            timeout(IO_LIMIT, rx.read_exact(&mut receipt))
                .await?
                .is_err()
        );
        Ok::<(), anyhow::Error>(())
    };
    let mut count = 0;
    let (sent, viewed) = tokio::join!(
        peer,
        view_h264_diagnostic(
            &viewer,
            host.id(),
            std::slice::from_ref(&addr),
            profile(),
            |_| {
                count += 1;
                ready(Ok(()))
            }
        )
    );
    sent?;
    assert!(viewed.is_err());
    assert_eq!(count, 0);
    viewer.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn separate_alpn_and_non_loopback_routes_are_rejected() -> Result<()> {
    assert!(
        bind_h264_diagnostic(SecretKey::generate(), "0.0.0.0:0".parse()?)
            .await
            .is_err()
    );
    let (host, viewer, addr) = endpoints().await?;
    assert!(
        view_h264_diagnostic(
            &viewer,
            host.id(),
            &["192.0.2.1:45000".parse()?],
            profile(),
            |_| ready(Ok(()))
        )
        .await
        .is_err()
    );
    // A v8 client cannot negotiate the H.264-only endpoint.
    let client = timeout(
        IO_LIMIT,
        viewer.connect(EndpointAddr::new(host.id()).with_ip_addr(addr), ALPN),
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

#[tokio::test]
async fn early_local_timer_does_not_grant_next_capture() -> Result<()> {
    struct Early;
    impl PacerWait for Early {
        async fn wait(&mut self, _: Duration) -> Result<()> {
            Ok(())
        }
    }
    let (host, viewer, addr) = endpoints().await?;
    // One second between samples ensures the intentionally early timer
    // cannot be masked by normal loopback processing time.
    let p = Profile::new(540, 1, 3, 8_000_000).unwrap();
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        let req = requests.recv().await.context("Missing demand")?;
        ensure!(
            req.reply
                .send(Ok(frame(1, true, SLOW_IDR, req.buffer)?))
                .is_ok(),
            "Canceled"
        );
        assert!(timeout(IO_LIMIT, requests.recv()).await?.is_none());
        Ok::<(), anyhow::Error>(())
    };
    let mut timer = Early;
    let (served, viewed, produced) = tokio::join!(
        serve_h264_diagnostic_with_waiter(&host, viewer.id(), p, source, &mut timer),
        view_h264_diagnostic(&viewer, host.id(), std::slice::from_ref(&addr), p, |_| {
            ready(Ok(()))
        }),
        producer
    );
    produced?;
    assert!(served.is_err() && viewed.is_err());
    viewer.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn encoder_packetizer_reuses_authenticated_demand_and_receipts() -> Result<()> {
    use h264::{Framing, Packetizer};
    let (host, viewer, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        let mut packetizer = Packetizer::new(profile());
        let inputs: [(Framing, bool, &[u8]); 3] = [
            (Framing::AnnexB, true, &IDR[5..]),
            (Framing::LengthPrefixed4, false, &[0, 0, 0, 2, 0x61, 0xe0]),
            (Framing::AnnexB, true, &IDR[IDR.len() - 5..]),
        ];
        for (framing, key, data) in inputs {
            let req = requests.recv().await.context("Missing capture demand")?;
            let mut output = req.buffer;
            packetizer
                .packetize(framing, key, data, &mut output)
                .map_err(|err| anyhow::anyhow!("Packetizer rejected fixture: {err:?}"))?;
            ensure!(
                req.reply.send(Ok(Capture::Frame(output))).is_ok(),
                "Capture canceled"
            );
        }
        let req = requests.recv().await.context("Missing end demand")?;
        ensure!(req.reply.send(Ok(Capture::End)).is_ok(), "Capture canceled");
        Ok::<(), anyhow::Error>(())
    };
    let mut observed = Vec::new();
    let (served, viewed, produced) = tokio::join!(
        serve_h264_diagnostic(&host, viewer.id(), profile(), source),
        view_h264_diagnostic(
            &viewer,
            host.id(),
            std::slice::from_ref(&addr),
            profile(),
            |f| {
                let result = (|| {
                    let au = h264::unwrap(&f.payload).map_err(|err| anyhow::anyhow!("{err:?}"))?;
                    let expected = if au.keyframe { IDR } else { P };
                    ensure!(au.annex_b == expected, "Normalized bytes changed");
                    observed.push((f.record_sequence, au.sequence, au.keyframe));
                    Ok(())
                })();
                ready(result)
            }
        ),
        producer
    );
    produced?;
    let served = served?;
    assert_eq!(served, viewed?);
    assert_eq!(served.frames, 3);
    assert_eq!(observed, vec![(1, 1, true), (2, 2, false), (3, 3, true)]);
    viewer.close().await;
    host.close().await;
    Ok(())
}
