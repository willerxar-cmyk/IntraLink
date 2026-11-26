use super::*;
use crate::{bind, send_probe};
use intralink_core::desktop::parse_request;
use intralink_core::desktop::{MAX_IN_FLIGHT_RECORDS, ack};
use std::future::ready;

fn config() -> VideoConfig {
    VideoConfig::new(540, 10, 75, 3).unwrap()
}

async fn endpoints() -> Result<(Endpoint, Endpoint, SocketAddr)> {
    let host = bind_view(SecretKey::generate(), "127.0.0.1:0".parse()?).await?;
    let controller = bind(SecretKey::generate(), "127.0.0.1:0".parse()?, false).await?;
    let addr = host
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("Missing loopback")?;
    Ok((host, controller, addr))
}

#[tokio::test]
async fn rejected_identity_never_requests_capture() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel(1);
    let unrelated = SecretKey::generate().public();
    let (served, viewed) = tokio::join!(
        serve_view(&host, unrelated, config(), source),
        view(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Jpeg,
            |_| ready(Ok(()))
        )
    );
    assert!(served.is_err());
    assert!(viewed.is_err());
    assert!(requests.try_recv().is_err());
    controller.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn authorized_source_can_stop_without_capture() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        let req = requests.recv().await.context("Missing capture request")?;
        assert!(req.reply.send(Ok(Capture::End)).is_ok());
        Ok::<(), anyhow::Error>(())
    };
    let (served, viewed, produced) = tokio::join!(
        serve_view(&host, controller.id(), config(), source),
        view(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Jpeg,
            |_| ready(Err(anyhow::anyhow!("Unexpected frame")))
        ),
        producer
    );
    produced?;
    assert_eq!(served?, Stats::default());
    assert_eq!(viewed?, Stats::default());
    controller.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn changed_and_repeat_records_preserve_sequence_and_recycle_buffer() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        for i in 0..3 {
            let req = requests.recv().await.context("Missing capture request")?;
            let mut buffer = req.buffer;
            assert!(buffer.is_empty());
            let capture = match i {
                0 => {
                    buffer.extend_from_slice(&[10, 20, 30]);
                    Capture::Frame(buffer)
                }
                1 => {
                    assert!(buffer.capacity() >= 3);
                    Capture::Repeat(buffer)
                }
                _ => Capture::End,
            };
            assert!(req.reply.send(Ok(capture)).is_ok());
        }
        Ok::<(), anyhow::Error>(())
    };
    let mut displayed = 0;
    let (served, viewed, produced) = tokio::join!(
        serve_view(&host, controller.id(), config(), source),
        view(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Jpeg,
            |frame| {
                displayed += 1;
                ready(
                    if frame.payload == [10, 20, 30] && frame.config == config() {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("Frame corrupted"))
                    },
                )
            }
        ),
        producer
    );
    produced?;
    assert_eq!(displayed, 1);
    assert_eq!(
        served?,
        Stats {
            frames: 1,
            repeats: 1,
            no_updates: 0,
            media_bytes: 3
        }
    );
    assert_eq!(
        viewed?,
        Stats {
            frames: 1,
            repeats: 1,
            no_updates: 0,
            media_bytes: 3
        }
    );
    controller.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn repeat_before_first_frame_is_rejected() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        let req = requests.recv().await.context("Missing capture request")?;
        assert!(req.reply.send(Ok(Capture::Repeat(req.buffer))).is_ok());
        Ok::<(), anyhow::Error>(())
    };
    let (served, viewed, produced) = tokio::join!(
        serve_view(&host, controller.id(), config(), source),
        view(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Jpeg,
            |_| ready(Err(anyhow::anyhow!("Unexpected frame")))
        ),
        producer
    );
    produced?;
    assert!(served.is_err());
    assert!(viewed.is_err());
    controller.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn diagnostic_protocol_cannot_open_view_listener() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let sent = send_probe(&controller, host.id(), addr).await;
    controller.close().await;
    host.close().await;
    assert!(sent.is_err());
    Ok(())
}

#[tokio::test]
async fn wrong_ack_cannot_exceed_credit_window() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        for _ in 0..MAX_IN_FLIGHT_RECORDS {
            let req = requests.recv().await.context("Missing capture request")?;
            let mut buffer = req.buffer;
            buffer.extend_from_slice(&[10, 20, 30]);
            assert!(req.reply.send(Ok(Capture::Frame(buffer))).is_ok());
        }
        // A wrong ACK cannot release credit for another capture.
        assert!(
            timeout(Duration::from_secs(10), requests.recv())
                .await?
                .is_none()
        );
        Ok::<(), anyhow::Error>(())
    };
    let bad_peer = async {
        let connection = timeout(
            HANDSHAKE_LIMIT,
            controller.connect(EndpointAddr::new(host.id()).with_ip_addr(addr), ALPN),
        )
        .await??;
        let (mut tx, mut rx) = connection.open_bi().await?;
        tx.write_all(&request(CodecMode::Jpeg)).await?;
        let mut config_bytes = [0; CONFIG_BYTES];
        timeout(IO_LIMIT, rx.read_exact(&mut config_bytes)).await??;
        assert_eq!(VideoConfig::decode(config_bytes), Some(config()));
        for _ in 0..MAX_IN_FLIGHT_RECORDS {
            let mut header = [0; 4];
            timeout(IO_LIMIT, rx.read_exact(&mut header)).await??;
            assert_eq!(parse_length(header), Some(Record::Frame(3)));
            let mut data = [0; 3];
            timeout(IO_LIMIT, rx.read_exact(&mut data)).await??;
            assert_eq!(data, [10, 20, 30]);
        }
        tx.write_all(&2u32.to_be_bytes()).await?;
        let _ = timeout(IO_LIMIT, connection.closed()).await?;
        Ok::<(), anyhow::Error>(())
    };
    let (served, produced, client) = tokio::join!(
        serve_view(&host, controller.id(), config(), source),
        producer,
        bad_peer
    );
    produced?;
    client?;
    assert!(served.is_err());
    controller.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn credit_allows_bounded_records_before_ack_and_drains_at_end() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let profile = VideoConfig::new(540, 30, 75, 10).unwrap();
    let (blocked, blocked_rx) = oneshot::channel();
    let producer = async {
        for i in 0..MAX_IN_FLIGHT_RECORDS {
            let req = timeout(IO_LIMIT, requests.recv())
                .await?
                .context("Missing capture request")?;
            let expected_pressure = match i {
                0..=2 => ReceiptPressure::Low,
                3..=5 => ReceiptPressure::Neutral,
                _ => ReceiptPressure::High,
            };
            assert_eq!(req.receipt_pressure, expected_pressure);
            let mut data = req.buffer;
            data.push(i as u8);
            assert!(req.reply.send(Ok(Capture::Frame(data))).is_ok());
        }
        // Client waits for this observation before releasing one credit.
        assert!(
            timeout(Duration::from_millis(200), requests.recv())
                .await
                .is_err()
        );
        assert!(blocked.send(()).is_ok());
        let req = timeout(IO_LIMIT, requests.recv())
            .await?
            .context("Missing next capture")?;
        // Full-window pressure survives the wait for the next valid receipt.
        assert_eq!(req.receipt_pressure, ReceiptPressure::High);
        let mut data = req.buffer;
        data.push(MAX_IN_FLIGHT_RECORDS as u8);
        assert!(req.reply.send(Ok(Capture::Frame(data))).is_ok());
        let req = timeout(IO_LIMIT, requests.recv())
            .await?
            .context("Missing stop request")?;
        assert!(req.reply.send(Ok(Capture::End)).is_ok());
        Ok::<(), anyhow::Error>(())
    };
    let peer = async {
        let connection = timeout(
            HANDSHAKE_LIMIT,
            controller.connect(EndpointAddr::new(host.id()).with_ip_addr(addr), ALPN),
        )
        .await??;
        let (mut tx, mut rx) = connection.open_bi().await?;
        tx.write_all(&request(CodecMode::Jpeg)).await?;
        let mut configuration = [0; CONFIG_BYTES];
        timeout(IO_LIMIT, rx.read_exact(&mut configuration)).await??;
        assert_eq!(VideoConfig::decode(configuration), Some(profile));
        for i in 0..MAX_IN_FLIGHT_RECORDS {
            let mut header = [0; 4];
            timeout(IO_LIMIT, rx.read_exact(&mut header)).await??;
            assert_eq!(parse_length(header), Some(Record::Frame(1)));
            let mut data = [0; 1];
            timeout(IO_LIMIT, rx.read_exact(&mut data)).await??;
            assert_eq!(data, [i as u8]);
        }
        timeout(IO_LIMIT, blocked_rx).await??;
        tx.write_all(&ack(1).unwrap()).await?;
        let mut header = [0; 4];
        timeout(IO_LIMIT, rx.read_exact(&mut header)).await??;
        assert_eq!(parse_length(header), Some(Record::Frame(1)));
        let mut data = [0; 1];
        timeout(IO_LIMIT, rx.read_exact(&mut data)).await??;
        assert_eq!(data, [MAX_IN_FLIGHT_RECORDS as u8]);
        // Release remaining credit and verify normal termination.
        for sequence in 2..=MAX_IN_FLIGHT_RECORDS + 1 {
            tx.write_all(&ack(sequence).unwrap()).await?;
        }
        let mut end = [0; 4];
        timeout(IO_LIMIT, rx.read_exact(&mut end)).await??;
        assert_eq!(parse_length(end), Some(Record::End));
        timeout(IO_LIMIT, rx.read_to_end(0)).await??;
        tx.finish()?;
        connection.close(0u32.into(), b"test complete");
        Ok::<(), anyhow::Error>(())
    };
    let (served, produced, client) = tokio::join!(
        serve_view(&host, controller.id(), profile, source),
        producer,
        peer
    );
    produced?;
    client?;
    assert_eq!(
        served?,
        Stats {
            frames: MAX_IN_FLIGHT_RECORDS + 1,
            repeats: 0,
            no_updates: 0,
            media_bytes: (MAX_IN_FLIGHT_RECORDS + 1) as u64
        }
    );
    controller.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn mismatched_media_mode_never_requests_capture() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel(1);
    let (served, viewed) = tokio::join!(
        serve_view(&host, controller.id(), config(), source),
        view(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Tiles,
            |_| ready(Ok(()))
        )
    );
    assert!(served.is_err());
    assert!(viewed.is_err());
    assert!(requests.try_recv().is_err());
    controller.close().await;
    host.close().await;
    Ok(())
}

#[test]
fn payload_preparation_preserves_allocation_and_rejects_invalid_sizes() -> Result<()> {
    let mut buffer = Vec::with_capacity(1024);
    buffer.extend_from_slice(&[1, 2, 3]);
    let pointer = buffer.as_ptr();
    prepare_payload(&mut buffer, 512)?;
    assert_eq!(buffer.as_ptr(), pointer);
    assert_eq!(buffer.len(), 512);
    assert!(buffer.iter().all(|byte| *byte == 0));
    assert!(prepare_payload(&mut buffer, 0).is_err());
    assert!(prepare_payload(&mut buffer, intralink_core::desktop::MAX_FRAME_BYTES + 1).is_err());
    assert_eq!(buffer.len(), 512);
    let mut oversized = Vec::with_capacity(intralink_core::desktop::MAX_FRAME_BYTES + 1);
    prepare_payload(&mut oversized, 64)?;
    assert_eq!(oversized.len(), 64);
    assert!(oversized.capacity() <= intralink_core::desktop::MAX_FRAME_BYTES);
    Ok(())
}

#[tokio::test]
async fn receiver_reuses_consumed_payload_across_ordered_records() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let (recycle, mut pool) = mpsc::channel::<Vec<u8>>(1);
    let producer = async {
        for value in [1u8, 2, 3] {
            let req = requests.recv().await.context("Missing capture request")?;
            let mut data = req.buffer;
            data.resize(256, value);
            assert!(req.reply.send(Ok(Capture::Frame(data))).is_ok());
        }
        let req = requests.recv().await.context("Missing stop request")?;
        assert!(req.reply.send(Ok(Capture::End)).is_ok());
        Ok::<(), anyhow::Error>(())
    };
    let mut sequence = 0usize;
    let mut previous_address = None;
    let (served, viewed, produced) = tokio::join!(
        serve_view(&host, controller.id(), config(), source),
        view_with_buffers(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Jpeg,
            || pool.try_recv().unwrap_or_default(),
            |frame| {
                sequence += 1;
                assert_eq!(frame.sequence, sequence);
                assert!(
                    frame
                        .payload
                        .iter()
                        .all(|byte| usize::from(*byte) == sequence)
                );
                let address = frame.payload.as_ptr() as usize;
                if let Some(previous) = previous_address {
                    assert_eq!(address, previous);
                }
                previous_address = Some(address);
                ready(
                    recycle
                        .try_send(frame.payload)
                        .map_err(|_| anyhow::anyhow!("Pool unexpectedly full")),
                )
            },
        ),
        producer,
    );
    produced?;
    assert_eq!(
        served?,
        Stats {
            frames: 3,
            repeats: 0,
            no_updates: 0,
            media_bytes: 768
        }
    );
    assert_eq!(
        viewed?,
        Stats {
            frames: 3,
            repeats: 0,
            no_updates: 0,
            media_bytes: 768
        }
    );
    assert_eq!(sequence, 3);
    controller.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn three_sessions_are_isolated_when_one_host_refuses() -> Result<()> {
    async fn session(
        controller_key: SecretKey,
        authorized: bool,
        marker: u8,
        priority: ViewPriority,
    ) -> Result<()> {
        let host = bind_view(SecretKey::generate(), "127.0.0.1:0".parse()?).await?;
        let controller = bind(controller_key, "127.0.0.1:0".parse()?, false).await?;
        let addr = host
            .bound_sockets()
            .into_iter()
            .find(SocketAddr::is_ipv4)
            .context("Missing loopback")?;
        let approved = if authorized {
            controller.id()
        } else {
            SecretKey::generate().public()
        };
        let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
        let producer = async {
            if !authorized {
                assert!(timeout(IO_LIMIT, requests.recv()).await?.is_none());
            } else {
                let request = requests.recv().await.context("Missing frame request")?;
                let mut buffer = request.buffer;
                buffer.push(marker);
                assert!(request.reply.send(Ok(Capture::Frame(buffer))).is_ok());
                let request = requests.recv().await.context("Missing end request")?;
                assert!(request.reply.send(Ok(Capture::End)).is_ok());
            }
            Ok::<(), anyhow::Error>(())
        };
        let (served, viewed, produced) = tokio::join!(
            serve_view(&host, approved, config(), source),
            view_with_policy(
                &controller,
                host.id(),
                std::slice::from_ref(&addr),
                CodecMode::Jpeg,
                Vec::new,
                |frame| ready(if frame.payload == [marker] {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Cross-session frame"))
                }),
                || priority
            ),
            producer,
        );
        controller.close().await;
        host.close().await;
        produced?;
        if authorized {
            let expected = Stats {
                frames: 1,
                repeats: 0,
                no_updates: 0,
                media_bytes: 1,
            };
            assert_eq!(served?, expected);
            assert_eq!(viewed?, expected);
        } else {
            assert!(served.is_err());
            assert!(viewed.is_err());
        }
        Ok(())
    }
    let controller = SecretKey::generate();
    let (one, two, three) = tokio::join!(
        session(controller.clone(), true, 1, ViewPriority::Foreground),
        session(controller.clone(), false, 2, ViewPriority::Background),
        session(controller, true, 3, ViewPriority::Background),
    );
    one?;
    two?;
    three?;
    Ok(())
}

#[tokio::test]
async fn no_update_is_ordered_but_never_decoded_or_counted_as_repeat() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        for index in 0..5 {
            let req = requests.recv().await.context("Missing capture request")?;
            let mut data = req.buffer;
            assert!(data.is_empty());
            let record = match index {
                0 | 3 => {
                    data.push(index as u8);
                    Capture::Frame(data)
                }
                1 => Capture::NoUpdate(data),
                2 => Capture::Repeat(data),
                _ => Capture::End,
            };
            assert!(req.reply.send(Ok(record)).is_ok());
        }
        Ok::<(), anyhow::Error>(())
    };
    let mut frames = Vec::new();
    let mut observations = Vec::new();
    let (served, viewed, produced) = tokio::join!(
        serve_view(&host, controller.id(), config(), source),
        view_with_observations(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Jpeg,
            Vec::new,
            |frame| {
                frames.push((frame.sequence, frame.payload));
                ready(Ok(()))
            },
            |record| {
                observations.push(record);
                ViewPriority::Foreground
            }
        ),
        producer
    );
    produced?;
    let expected = Stats {
        frames: 2,
        repeats: 1,
        no_updates: 1,
        media_bytes: 2,
    };
    assert_eq!(served?, expected);
    assert_eq!(viewed?, expected);
    assert_eq!(frames, vec![(1, vec![0]), (4, vec![3])]);
    assert_eq!(
        observations,
        vec![
            Record::Frame(1),
            Record::NoUpdate,
            Record::Repeat,
            Record::Frame(1)
        ]
    );
    controller.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn no_update_before_first_frame_is_rejected_by_sender() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        let req = requests.recv().await.context("Missing capture request")?;
        assert!(req.reply.send(Ok(Capture::NoUpdate(req.buffer))).is_ok());
        Ok::<(), anyhow::Error>(())
    };
    let (served, viewed, produced) = tokio::join!(
        serve_view(&host, controller.id(), config(), source),
        view(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Jpeg,
            |_| ready(Err(anyhow::anyhow!("Unexpected frame")))
        ),
        producer
    );
    produced?;
    assert!(served.is_err());
    assert!(viewed.is_err());
    controller.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn no_update_before_first_frame_is_rejected_by_receiver() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let peer = async {
        let incoming = timeout(IO_LIMIT, host.accept())
            .await?
            .context("Missing connection")?;
        let connection = incoming.await?;
        let (mut tx, mut rx) = connection.accept_bi().await?;
        let mut bytes = [0; REQUEST_BYTES];
        rx.read_exact(&mut bytes).await?;
        assert_eq!(parse_request(&bytes), Some(CodecMode::Jpeg));
        tx.write_all(&config().encode()).await?;
        tx.write_all(&NO_UPDATE).await?;
        let _ = timeout(IO_LIMIT, connection.closed()).await?;
        Ok::<(), anyhow::Error>(())
    };
    let (sent, viewed) = tokio::join!(
        peer,
        view(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Jpeg,
            |_| ready(Err(anyhow::anyhow!("Unexpected frame")))
        )
    );
    sent?;
    assert!(viewed.is_err());
    controller.close().await;
    host.close().await;
    Ok(())
}
