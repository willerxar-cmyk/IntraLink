use super::*;
use intralink_core::input::{Button, Focus, Key};
use tokio::task::JoinHandle;

struct Host {
    endpoint: Endpoint,
    address: SocketAddr,
    transport: JoinHandle<Result<u32>>,
    consumer: JoinHandle<Result<Vec<Event>>>,
}

async fn host(controller: EndpointId) -> Result<Host> {
    let endpoint = bind_input(SecretKey::generate(), "127.0.0.1:0".parse()?).await?;
    let address = endpoint
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("No loopback")?;
    let (tx, mut rx) = mpsc::channel::<Delivery>(1);
    let listener = endpoint.clone();
    let transport = tokio::spawn(async move { serve_input(&listener, controller, 20, tx).await });
    let consumer = tokio::spawn(async move {
        let mut state = State::default();
        let mut events = Vec::new();
        while let Some(delivery) = rx.recv().await {
            state.commit(delivery.record)?;
            events.push(delivery.record.event);
            delivery
                .accepted
                .send(Ok(()))
                .map_err(|_| anyhow::anyhow!("Dropped acceptance"))?;
        }
        ensure!(
            !state.active() && state.releases().is_empty(),
            "Input remained held after End"
        );
        Ok(events)
    });
    Ok(Host {
        endpoint,
        address,
        transport,
        consumer,
    })
}

#[tokio::test]
async fn three_authenticated_hosts_keep_independent_input_and_release_before_switch() -> Result<()>
{
    multi_host(3).await
}

#[tokio::test]
async fn eight_authenticated_input_hosts_share_one_endpoint() -> Result<()> {
    multi_host(intralink_core::input::MAX_SLOTS).await
}

async fn multi_host(count: usize) -> Result<()> {
    let controller = crate::bind_workspace_controller(SecretKey::generate()).await?;
    let mut hosts = Vec::new();
    let mut sessions = Vec::new();
    for _ in 0..count {
        let host = host(controller.id()).await?;
        let session =
            InputConnection::connect(&controller, host.endpoint.id(), &[host.address]).await?;
        hosts.push(host);
        sessions.push(session);
    }
    let mut focus = Focus::default();
    let key = Key::new(4).unwrap();
    for target in 0..count {
        if let Some(previous) = focus.begin(Some(target))? {
            sessions[previous].send(Event::Deactivate).await?;
        }
        assert_eq!(focus.released()?, Some(target));
        sessions[target].send(Event::Activate).await?;
        focus.activated(Some(target))?;
        sessions[target]
            .send(Event::Move {
                x: 100 + target as u16,
                y: 200,
            })
            .await?;
        sessions[target]
            .send(Event::Key { key, down: true })
            .await?;
        sessions[target]
            .send(Event::Button {
                button: Button::Left,
                down: true,
            })
            .await?;
        sessions[target].send(Event::Heartbeat).await?;
    }
    for session in &mut sessions {
        session.send(Event::End).await?;
    }
    drop(sessions);
    for (index, host) in hosts.into_iter().enumerate() {
        let records = host.transport.await??;
        let events = host.consumer.await??;
        assert_eq!(records as usize, events.len());
        assert_eq!(events[0], Event::Activate);
        assert_eq!(
            events[1],
            Event::Move {
                x: 100 + index as u16,
                y: 200
            }
        );
        assert_eq!(events[2], Event::Key { key, down: true });
        assert_eq!(events.last(), Some(&Event::End));
        if index + 1 < count {
            assert!(events.contains(&Event::Deactivate));
        }
        host.endpoint.close().await;
    }
    controller.close().await;
    Ok(())
}

#[tokio::test]
async fn wrong_identity_never_reaches_input_sink() -> Result<()> {
    let listener = bind_input(SecretKey::generate(), "127.0.0.1:0".parse()?).await?;
    let attacker = crate::bind_workspace_controller(SecretKey::generate()).await?;
    let approved = SecretKey::generate().public();
    let address = listener
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("No loopback")?;
    let (tx, mut rx) = mpsc::channel(1);
    let host = listener.clone();
    let served = tokio::spawn(async move { serve_input(&host, approved, 20, tx).await });
    assert!(
        InputConnection::connect(&attacker, listener.id(), &[address])
            .await
            .is_err()
    );
    assert!(served.await?.is_err());
    assert!(rx.recv().await.is_none());
    attacker.close().await;
    listener.close().await;
    Ok(())
}

#[tokio::test]
async fn malformed_sequence_or_inactive_input_never_reaches_local_sink() -> Result<()> {
    for event in [Event::Move { x: 1, y: 2 }, Event::Activate] {
        let listener = bind_input(SecretKey::generate(), "127.0.0.1:0".parse()?).await?;
        let controller = crate::bind_workspace_controller(SecretKey::generate()).await?;
        let address = listener
            .bound_sockets()
            .into_iter()
            .find(SocketAddr::is_ipv4)
            .context("No loopback")?;
        let (tx, mut rx) = mpsc::channel(1);
        let host = listener.clone();
        let approved = controller.id();
        let served = tokio::spawn(async move { serve_input(&host, approved, 20, tx).await });
        let connection = controller
            .connect(EndpointAddr::new(listener.id()).with_ip_addr(address), ALPN)
            .await?;
        let (mut writer, mut reader) = connection.open_bi().await?;
        writer.write_all(HELLO).await?;
        let mut ready = [0; 10];
        reader.read_exact(&mut ready).await?;
        assert_eq!(&ready[..8], READY);
        let sequence = if event == Event::Activate { 2 } else { 1 };
        writer
            .write_all(&Record { sequence, event }.encode()?)
            .await?;
        assert!(served.await?.is_err());
        assert!(rx.recv().await.is_none());
        controller.close().await;
        listener.close().await;
    }
    Ok(())
}

#[tokio::test]
async fn local_rejection_is_not_acknowledged() -> Result<()> {
    let listener = bind_input(SecretKey::generate(), "127.0.0.1:0".parse()?).await?;
    let controller = crate::bind_workspace_controller(SecretKey::generate()).await?;
    let address = listener
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("No loopback")?;
    let (tx, mut rx) = mpsc::channel::<Delivery>(1);
    let host = listener.clone();
    let approved = controller.id();
    let served = tokio::spawn(async move { serve_input(&host, approved, 20, tx).await });
    let denied = tokio::spawn(async move {
        let event = rx.recv().await.context("No event")?;
        ensure!(event.record.event == Event::Activate, "Unexpected event");
        event
            .accepted
            .send(Err(anyhow::anyhow!("Local owner canceled")))
            .map_err(|_| anyhow::anyhow!("Canceled reply"))?;
        Ok::<(), anyhow::Error>(())
    });
    let mut session = InputConnection::connect(&controller, listener.id(), &[address]).await?;
    assert!(session.send(Event::Activate).await.is_err());
    assert!(!session.active());
    denied.await??;
    assert!(served.await?.is_err());
    controller.close().await;
    listener.close().await;
    Ok(())
}

#[tokio::test]
async fn disconnected_controller_closes_consumer_with_owned_release_plan() -> Result<()> {
    let listener = bind_input(SecretKey::generate(), "127.0.0.1:0".parse()?).await?;
    let controller = crate::bind_workspace_controller(SecretKey::generate()).await?;
    let address = listener
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("No loopback")?;
    let (tx, mut rx) = mpsc::channel::<Delivery>(1);
    let host = listener.clone();
    let approved = controller.id();
    let served = tokio::spawn(async move { serve_input(&host, approved, 20, tx).await });
    let consumer = tokio::spawn(async move {
        let mut state = State::default();
        while let Some(delivery) = rx.recv().await {
            state.commit(delivery.record)?;
            delivery
                .accepted
                .send(Ok(()))
                .map_err(|_| anyhow::anyhow!("Dropped acceptance"))?;
        }
        Ok::<_, anyhow::Error>(state.releases())
    });
    let mut session = InputConnection::connect(&controller, listener.id(), &[address]).await?;
    let key = Key::new(225).unwrap();
    session.send(Event::Activate).await?;
    session.send(Event::Key { key, down: true }).await?;
    session
        .send(Event::Button {
            button: Button::Left,
            down: true,
        })
        .await?;
    drop(session);
    assert!(timeout(Duration::from_secs(5), served).await??.is_err());
    let releases = consumer.await??;
    assert_eq!(
        releases,
        vec![
            Event::Key { key, down: false },
            Event::Button {
                button: Button::Left,
                down: false
            }
        ]
    );
    controller.close().await;
    listener.close().await;
    Ok(())
}
