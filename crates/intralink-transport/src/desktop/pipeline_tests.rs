//! Real authenticated QUIC loopback tests for the local two-demand window.
use super::*;
use crate::bind;
use intralink_core::desktop::ack;
use std::future::ready;

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

async fn next(requests: &mut mpsc::Receiver<CaptureRequest>) -> Result<CaptureRequest> {
    let mut request = timeout(IO_LIMIT, requests.recv())
        .await?
        .context("Missing local demand")?;
    let due = request
        .not_before()
        .context("Missing scheduled start grant")?;
    tokio::time::sleep_until(Instant::from_std(due)).await;
    request.begin_capture()?;
    Ok(request)
}

fn reply(request: CaptureRequest, value: u8) -> Result<()> {
    let mut bytes = request.buffer;
    ensure!(bytes.is_empty(), "Uncleared transport buffer");
    bytes.push(value);
    request
        .reply
        .send(Ok(Capture::Frame(bytes)))
        .map_err(|_| anyhow::anyhow!("Capture demand dropped"))
}

#[tokio::test]
async fn two_demands_overlap_and_out_of_order_completion_is_serialized() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let config = VideoConfig::new(540, 60, 75, 10).unwrap();
    let (source, mut requests) = mpsc::channel(1);
    let producer = async {
        reply(next(&mut requests).await?, 1)?;
        let second = next(&mut requests).await?;
        // Hold the second response until the third demand arrives.
        let third = next(&mut requests).await?;
        assert!(
            timeout(Duration::from_millis(100), requests.recv())
                .await
                .is_err()
        );
        reply(third, 3)?;
        reply(second, 2)?;
        let end = next(&mut requests).await?;
        ensure!(
            end.reply.send(Ok(Capture::End)).is_ok(),
            "End demand dropped"
        );
        // If an already reserved demand exists after End, it must be
        // canceled by the transport, never published.
        while let Some(mut extra) = requests.recv().await {
            extra.reply.closed().await;
        }
        Ok::<(), anyhow::Error>(())
    };
    let mut observed = Vec::new();
    let (served, viewed, produced) = tokio::join!(
        serve_view_pipelined(&host, controller.id(), config, source),
        view(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Jpeg,
            |frame| {
                observed.push((frame.sequence, frame.payload));
                ready(Ok(()))
            }
        ),
        producer
    );
    produced?;
    assert_eq!(served?.frames, 3);
    assert_eq!(viewed?.frames, 3);
    assert_eq!(observed, vec![(1, vec![1]), (2, vec![2]), (3, vec![3])]);
    controller.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn reserved_unsent_demands_cannot_expand_credit() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let config = VideoConfig::new(540, 60, 75, 10).unwrap();
    let (source, mut requests) = mpsc::channel(1);
    let (blocked, blocked_rx) = oneshot::channel();
    let producer = async {
        for value in 0..6 {
            reply(next(&mut requests).await?, value)?;
        }
        let seventh = next(&mut requests).await?;
        let eighth = next(&mut requests).await?;
        assert!(
            timeout(Duration::from_millis(120), requests.recv())
                .await
                .is_err()
        );
        reply(eighth, 7)?;
        reply(seventh, 6)?;
        assert!(
            timeout(Duration::from_millis(120), requests.recv())
                .await
                .is_err()
        );
        ensure!(blocked.send(()).is_ok(), "Missing credit test observation");
        reply(next(&mut requests).await?, 8)?;
        let end = next(&mut requests).await?;
        ensure!(end.reply.send(Ok(Capture::End)).is_ok(), "Missing end");
        while let Some(mut extra) = requests.recv().await {
            extra.reply.closed().await;
        }
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
        assert_eq!(VideoConfig::decode(configuration), Some(config));
        for value in 0..8 {
            let mut header = [0; 4];
            timeout(IO_LIMIT, rx.read_exact(&mut header)).await??;
            assert_eq!(parse_length(header), Some(Record::Frame(1)));
            let mut data = [0];
            timeout(IO_LIMIT, rx.read_exact(&mut data)).await??;
            assert_eq!(data, [value]);
        }
        timeout(IO_LIMIT, blocked_rx).await??;
        tx.write_all(&ack(1).unwrap()).await?;
        let mut header = [0; 4];
        timeout(IO_LIMIT, rx.read_exact(&mut header)).await??;
        assert_eq!(parse_length(header), Some(Record::Frame(1)));
        let mut data = [0];
        timeout(IO_LIMIT, rx.read_exact(&mut data)).await??;
        assert_eq!(data, [8]);
        for seq in 2..=9 {
            tx.write_all(&ack(seq).unwrap()).await?;
        }
        timeout(IO_LIMIT, rx.read_exact(&mut header)).await??;
        assert_eq!(parse_length(header), Some(Record::End));
        timeout(IO_LIMIT, rx.read_to_end(0)).await??;
        tx.finish()?;
        connection.close(0u32.into(), b"test complete");
        Ok::<(), anyhow::Error>(())
    };
    let (served, produced, received) = tokio::join!(
        serve_view_pipelined(&host, controller.id(), config, source),
        producer,
        peer
    );
    produced?;
    received?;
    assert_eq!(served?.frames, 9);
    controller.close().await;
    host.close().await;
    Ok(())
}

#[tokio::test]
async fn wrong_identity_never_reserves_pipeline_capture() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let config = VideoConfig::new(540, 60, 75, 3).unwrap();
    let (source, mut requests) = mpsc::channel(1);
    let (served, viewed) = tokio::join!(
        serve_view_pipelined(&host, SecretKey::generate().public(), config, source),
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
async fn capture_grant_rejects_early_duplicate_and_canceled_starts() -> Result<()> {
    let config = VideoConfig::new(540, 60, 75, 3).unwrap();
    let now = std::time::Instant::now();
    let (reply, response) = oneshot::channel();
    let (started, reported) = oneshot::channel();
    let due = now + Duration::from_secs(5);
    let mut request = CaptureRequest {
        buffer: Vec::new(),
        reply,
        receipt_pressure: ReceiptPressure::Neutral,
        desktop_config: Some(config),
        issued_at: now,
        permit: Some(CapturePermit {
            not_before: due,
            expires: due + Duration::from_secs(1),
            started: Some(started),
        }),
    };
    assert!(request.begin_capture().is_err());
    // Move this test-owned grant into its allowed window without relying on
    // a wall-clock duration or scheduler-specific timing threshold.
    let due = std::time::Instant::now();
    request.permit.as_mut().unwrap().not_before = due;
    let actual = request.begin_capture()?;
    assert_eq!(reported.await?, actual);
    assert!(actual >= due);
    assert!(request.begin_capture().is_err());
    drop(response);
    assert!(request.begin_capture().is_err());
    Ok(())
}

#[tokio::test]
async fn reply_without_start_is_rejected() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let config = VideoConfig::new(540, 60, 75, 3).unwrap();
    let (source, mut requests) = mpsc::channel(1);
    let producer = async {
        let request = timeout(IO_LIMIT, requests.recv())
            .await?
            .context("Missing request")?;
        // Send an otherwise valid response while dropping the unstarted grant.
        reply(request, 17)?;
        Ok::<(), anyhow::Error>(())
    };
    let (served, viewed, produced) = tokio::join!(
        serve_view_pipelined(&host, controller.id(), config, source),
        view(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Jpeg,
            |_| ready(Err(anyhow::anyhow!("Unstarted capture was transmitted")))
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
async fn reserved_start_is_based_on_actual_previous_start() -> Result<()> {
    let (host, controller, addr) = endpoints().await?;
    let config = VideoConfig::new(540, 10, 75, 5).unwrap();
    let (source, mut requests) = mpsc::channel(1);
    let producer = async {
        let first = next(&mut requests).await?;
        reply(first, 1)?;
        let mut second = timeout(IO_LIMIT, requests.recv())
            .await?
            .context("Missing second request")?;
        let due = second.not_before().context("Missing second grant")?;
        // Deliberately start late. The next grant must follow this actual
        // start, not the original scheduled time.
        tokio::time::sleep_until(Instant::from_std(due + Duration::from_millis(40))).await;
        let actual = second.begin_capture()?;
        let mut third = timeout(IO_LIMIT, requests.recv())
            .await?
            .context("Missing third reservation")?;
        let third_due = third.not_before().context("Missing third grant")?;
        assert!(third_due >= actual + Duration::from_millis(100));
        tokio::time::sleep_until(Instant::from_std(third_due)).await;
        third.begin_capture()?;
        reply(second, 2)?;
        reply(third, 3)?;
        let end = next(&mut requests).await?;
        ensure!(end.reply.send(Ok(Capture::End)).is_ok(), "Missing end");
        while let Some(mut extra) = requests.recv().await {
            extra.reply.closed().await;
        }
        Ok::<(), anyhow::Error>(())
    };
    let (served, viewed, produced) = tokio::join!(
        serve_view_pipelined(&host, controller.id(), config, source),
        view(
            &controller,
            host.id(),
            std::slice::from_ref(&addr),
            CodecMode::Jpeg,
            |_| ready(Ok(()))
        ),
        producer
    );
    produced?;
    assert_eq!(served?.frames, 3);
    assert_eq!(viewed?.frames, 3);
    controller.close().await;
    host.close().await;
    Ok(())
}
