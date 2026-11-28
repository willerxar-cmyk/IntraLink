//! Actual unified controller workers over pinned loopback transports and
//! generated media/input records. No native capture, real input or injection.
use super::*;
use intralink_core::{
    Role,
    adaptive::ViewProtocol,
    desktop::{CodecMode, VideoConfig},
    input::State,
    media::{MAX_DELTA_BYTES, encode_keyframe},
};
use intralink_transport::{
    desktop::{Capture, CaptureRequest, bind_view_with_protocol, serve_view_with_protocol},
    input::{Delivery, bind_input_geometry, serve_input_geometry},
};
use intralink_vault::Profile;
use std::net::SocketAddr;
use tokio::sync::mpsc;

fn address(endpoint: &Endpoint) -> Result<SocketAddr> {
    endpoint
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("Missing loopback")
}

async fn generated_event(slot: &Slot, event: Event) -> Result<()> {
    let (tx, rx) = oneshot::channel();
    slot.pulse()?;
    slot.queue(event, Some(tx))?;
    timeout(Duration::from_secs(2), rx).await??;
    Ok(())
}

async fn frame(
    viewer: &mut Viewer,
    requests: &mut mpsc::Receiver<CaptureRequest>,
    config: VideoConfig,
    sequence: usize,
    color: [u8; 3],
) -> Result<()> {
    let request = timeout(Duration::from_secs(5), requests.recv())
        .await?
        .context("Missing generated screen request")?;
    assert_eq!(request.desktop_config, Some(config));
    let mut payload = request.buffer;
    let pixels = config.width() as usize * config.height() as usize;
    assert!(
        encode_keyframe(config, &color.repeat(pixels), &mut payload, MAX_DELTA_BYTES)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?
    );
    ensure!(
        request.reply.send(Ok(Capture::Frame(payload))).is_ok(),
        "Fixture capture stopped"
    );
    timeout(Duration::from_secs(5), async {
        loop {
            viewer.refresh_media()?;
            if viewer.sequence == sequence && !viewer.screen.stale() {
                break;
            }
            sleep(TICK).await;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await??;
    let rect = canvas::image_rect(viewer.frame.context("Missing decoded frame")?).unwrap();
    let expected = (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2]);
    assert_eq!(
        viewer.canvas.pixels[rect.y * canvas::WIDTH + rect.x],
        expected
    );
    Ok(())
}

async fn scenario(window: &mut Window, input_first: bool) -> Result<()> {
    let mut base = Profile::new(Role::Controller);
    let mut client = Profile::new(Role::Host);
    let screen_host = bind_view_with_protocol(
        client.secret_key(),
        "127.0.0.1:0".parse()?,
        ViewProtocol::Fixed,
    )
    .await?;
    let input_host = bind_input_geometry(client.secret_key(), "127.0.0.1:0".parse()?).await?;
    let sa = address(&screen_host)?;
    let ia = address(&input_host)?;
    let offer = client.create_invite(
        base.public_id(),
        Route::new("127.0.0.1", sa.port())?,
        1000,
        120,
    )?;
    let reply = base.accept_invite("fixture", &offer, 1001)?;
    client.confirm_invite("base", &reply, 1002)?;
    let approved = client.peer("base", Direction::Inbound)?.identity()?;
    let peer = base.peer("fixture", Direction::Outbound)?;
    let config = VideoConfig::new(540, 30, 75, 30)
        .context("Invalid config")?
        .with_mode(CodecMode::Compact);
    let target = Target {
        name: "fixture".into(),
        identity: peer.identity()?,
        route: peer.route().context("No fixture route")?.clone(),
        mode: ViewMode::new(CodecMode::Compact, ViewProtocol::Fixed),
    };
    let route = Route::new("127.0.0.1", ia.port())?;
    let secret = base.secret_key();
    let display = Display {
        width: 1920,
        height: 1080,
        scale: 100,
        rotation: 0,
    };
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let listener = screen_host.clone();
    let serve_screen = tokio::spawn(async move {
        serve_view_with_protocol(&listener, approved, config, source, ViewProtocol::Fixed).await
    });
    let (deliver, mut deliveries) = mpsc::channel::<Delivery>(1);
    let listener = input_host.clone();
    let serve_input = tokio::spawn(async move {
        serve_input_geometry(&listener, approved, 30, deliver, display).await
    });
    let consumer = tokio::spawn(async move {
        let mut state = State::default();
        let mut events = Vec::new();
        while let Some(delivery) = deliveries.recv().await {
            state.commit(delivery.record)?;
            if delivery.record.event != Event::Heartbeat {
                events.push(delivery.record.event);
            }
            ensure!(
                delivery.accepted.send(Ok(())).is_ok(),
                "Fixture acceptance canceled"
            );
        }
        assert!(!state.active() && state.releases().is_empty());
        Ok::<_, anyhow::Error>(events)
    });

    let mut viewer = Viewer::new();
    viewer.screen.start(&target, &secret).await?;
    let endpoint = bind_workspace_controller(secret.clone()).await?;
    // This explicit loopback constructor exists only in test builds; the
    // production worker always enforces private-LAN routes.
    viewer.input.slot = Some(crate::input_slots::fixture_slot(
        &endpoint,
        target.name.clone(),
        target.identity,
        ia,
    ));
    viewer.input.endpoint = Some(endpoint);
    viewer.input.status = Lane::Connecting;
    timeout(Duration::from_secs(5), async {
        while viewer.input.display().is_none() {
            viewer.input.reap().await;
            sleep(TICK).await;
        }
    })
    .await?;
    viewer.input.reap().await;
    assert_eq!(viewer.input.display(), Some(display));
    frame(&mut viewer, &mut requests, config, 1, [19, 31, 71]).await?;
    assert_eq!(viewer.gate.state(), InputState::Safe);
    // Generated local review fixture. No pointer or keyboard is sampled
    // and no native cursor/key action is performed in this test.
    viewer.verified = true;
    viewer.canvas.chrome(&target.name, viewer.chrome(window));
    window
        .update_with_buffer(&viewer.canvas.pixels, canvas::WIDTH, canvas::HEIGHT)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    viewer.painted_region = viewer.desired_region(window);
    let region = viewer.region(window).context("Missing verified region")?;
    assert!(viewer.gate.arm(region));
    let movement = Event::Move { x: 1234, y: 5678 };
    let slot = viewer.input.slot.as_ref().unwrap();
    generated_event(slot, Event::Activate).await?;
    generated_event(slot, movement).await?;
    assert!(viewer.gate.acknowledge(region));

    if input_first {
        assert!(
            viewer
                .handle(Action::Input, window, &target, &route, &secret)
                .await?
        );
        assert!(viewer.input.slot.is_none());
        assert_eq!(viewer.input.status, Lane::Ended);
        assert_eq!(viewer.gate.state(), InputState::Safe);
        assert!(viewer.screen.live());
        frame(&mut viewer, &mut requests, config, 2, [71, 29, 13]).await?;
        let request = timeout(Duration::from_secs(5), requests.recv())
            .await?
            .context("Missing generated screen end")?;
        ensure!(
            request.reply.send(Ok(Capture::End)).is_ok(),
            "Screen end canceled"
        );
        timeout(Duration::from_secs(5), async {
            while viewer.screen.worker.is_some() {
                viewer.screen.reap().await;
                sleep(TICK).await;
            }
        })
        .await?;
        assert_eq!(serve_screen.await??.frames, 2);
        serve_input.await??;
        assert_eq!(
            consumer.await??,
            vec![Event::Activate, movement, Event::Deactivate, Event::End]
        );
    } else {
        assert!(
            viewer
                .handle(Action::Screen, window, &target, &route, &secret)
                .await?
        );
        assert!(viewer.frame.is_none());
        assert_eq!(viewer.gate.state(), InputState::Safe);
        assert_eq!(viewer.input.display(), Some(display));
        // Closing the screen must not close the independent input endpoint.
        drop(requests);
        timeout(Duration::from_secs(5), async {
            while viewer.screen.worker.is_some() {
                viewer.screen.reap().await;
                sleep(TICK).await;
            }
        })
        .await?;
        let _ = serve_screen.await?;
        assert_eq!(viewer.input.display(), Some(display));
        let slot = viewer.input.slot.as_ref().unwrap();
        let second = Event::Move { x: 60000, y: 40000 };
        generated_event(slot, Event::Activate).await?;
        generated_event(slot, second).await?;
        generated_event(slot, Event::Deactivate).await?;
        viewer.input.end(window).await;
        serve_input.await??;
        assert_eq!(
            consumer.await??,
            vec![
                Event::Activate,
                movement,
                Event::Deactivate,
                Event::Activate,
                second,
                Event::Deactivate,
                Event::End
            ]
        );
    }
    viewer.input.close(false).await;
    viewer.screen.close().await;
    screen_host.close().await;
    input_host.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires native local graphical desktop; generated loopback media and memory input only"]
async fn native_unified_workers_keep_independent_stops() -> Result<()> {
    let mut window = Window::new(
        "IntraLink | Generated dual-session fixture",
        960,
        540,
        WindowOptions {
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    window.set_target_fps(0);
    for input_first in [true, false] {
        timeout(Duration::from_secs(20), scenario(&mut window, input_first)).await??;
    }
    println!(
        "NATIVE_UNIFIED_WORKERS_PASS both stop orders; real pinned QUIC; generated media and memory-only input"
    );
    Ok(())
}
