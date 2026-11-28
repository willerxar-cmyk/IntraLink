//! Exercise two actual independent QUIC capabilities using the same signed
//! peer identities on separate UDP ports. Generated video and memory input
//! sinks only; no native capture, pointer injection, or real user profile.
use anyhow::{Context, Result, ensure};
use intralink_core::{
    Role,
    adaptive::ViewProtocol,
    desktop::{CodecMode, VideoConfig, ViewPriority},
    display_geometry::Display,
    input::{Event, State},
    media::{MAX_DELTA_BYTES, apply_keyframe, encode_keyframe},
};
use intralink_transport::{
    bind_workspace_controller,
    desktop::{
        Capture, CaptureRequest, VideoFrame, ViewMode, bind_view_with_protocol,
        serve_view_with_protocol, view_with_observations,
    },
    input::{Delivery, InputConnection, bind_input_geometry, serve_input_geometry},
};
use intralink_vault::{Direction, Profile, Route};
use iroh::Endpoint;
use std::{net::SocketAddr, time::Duration};
use tokio::{sync::mpsc, time::timeout};

const WAIT: Duration = Duration::from_secs(30);

fn address(endpoint: &Endpoint) -> Result<SocketAddr> {
    endpoint
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("Missing loopback address")
}

async fn generated_frame(
    requests: &mut mpsc::Receiver<CaptureRequest>,
    frames: &mut mpsc::Receiver<VideoFrame>,
    config: VideoConfig,
    sequence: usize,
    color: [u8; 3],
) -> Result<()> {
    let request = timeout(WAIT, requests.recv())
        .await?
        .context("Missing independent screen request")?;
    assert_eq!(request.desktop_config, Some(config));
    let mut payload = request.buffer;
    let rgb = color.repeat(config.width() as usize * config.height() as usize);
    ensure!(
        encode_keyframe(config, &rgb, &mut payload, MAX_DELTA_BYTES)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?,
        "Keyframe budget"
    );
    ensure!(
        request.reply.send(Ok(Capture::Frame(payload))).is_ok(),
        "Screen request canceled"
    );
    let frame = timeout(WAIT, frames.recv())
        .await?
        .context("Screen disconnected unexpectedly")?;
    assert_eq!(frame.sequence, sequence);
    assert_eq!(frame.config, config);
    let mut canvas = Vec::new();
    apply_keyframe(config, &frame.payload, &mut canvas).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    assert_eq!(
        canvas.len(),
        config.width() as usize * config.height() as usize
    );
    let expected = (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2]);
    assert!(canvas.iter().all(|pixel| *pixel == expected));
    Ok(())
}

async fn both(input_first: bool) -> Result<()> {
    let mut base = Profile::new(Role::Controller);
    let mut client = Profile::new(Role::Host);
    let screen_host = bind_view_with_protocol(
        client.secret_key(),
        "127.0.0.1:0".parse()?,
        ViewProtocol::Fixed,
    )
    .await?;
    let input_host = bind_input_geometry(client.secret_key(), "127.0.0.1:0".parse()?).await?;
    let screen_address = address(&screen_host)?;
    let input_address = address(&input_host)?;
    assert_ne!(screen_address.port(), input_address.port());
    assert_eq!(screen_host.id(), input_host.id());

    // Exactly the original bilateral identity grant; input port remains a
    // session-local alternate route, not a second or elevated grant.
    let offer = client.create_invite(
        base.public_id(),
        Route::new("127.0.0.1", screen_address.port())?,
        1000,
        120,
    )?;
    let reply = base.accept_invite("client", &offer, 1001)?;
    client.confirm_invite("base", &reply, 1002)?;
    let approved = client.peer("base", Direction::Inbound)?.identity()?;
    let expected = base.peer("client", Direction::Outbound)?.identity()?;
    assert_eq!(
        base.peer("client", Direction::Outbound)?
            .route()
            .context("Missing saved route")?
            .port(),
        screen_address.port()
    );

    let screen_base = bind_workspace_controller(base.secret_key()).await?;
    let input_base = bind_workspace_controller(base.secret_key()).await?;
    assert_eq!(screen_base.id(), input_base.id());
    assert_eq!(screen_base.id(), approved);
    assert_eq!(screen_host.id(), expected);
    let config = VideoConfig::new(540, 30, 75, 60)
        .context("Invalid video config")?
        .with_mode(CodecMode::Compact);
    let display = Display {
        width: 1920,
        height: 1080,
        scale: 100,
        rotation: 0,
    };

    let (capture, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let (frame_tx, mut frames) = mpsc::channel::<VideoFrame>(1);
    let video_listener = screen_host.clone();
    let video_task = tokio::spawn(async move {
        serve_view_with_protocol(
            &video_listener,
            approved,
            config,
            capture,
            ViewProtocol::Fixed,
        )
        .await
    });
    let video_controller = screen_base.clone();
    let view_task = tokio::spawn(async move {
        view_with_observations(
            &video_controller,
            expected,
            &[screen_address],
            ViewMode::new(CodecMode::Compact, ViewProtocol::Fixed),
            Vec::new,
            |frame| {
                let sender = frame_tx.clone();
                async move {
                    sender
                        .send(frame)
                        .await
                        .context("Fixture frame consumer closed")
                }
            },
            |_| ViewPriority::Foreground,
        )
        .await
    });

    let (deliver, mut sink) = mpsc::channel::<Delivery>(1);
    let input_listener = input_host.clone();
    let input_task = tokio::spawn(async move {
        serve_input_geometry(&input_listener, approved, 60, deliver, display).await
    });
    let consumer = tokio::spawn(async move {
        let mut state = State::default();
        let mut records = Vec::new();
        while let Some(delivery) = sink.recv().await {
            state.commit(delivery.record)?;
            records.push(delivery.record.event);
            ensure!(
                delivery.accepted.send(Ok(())).is_ok(),
                "Input acceptance canceled"
            );
        }
        assert!(!state.active() && state.releases().is_empty());
        Ok::<_, anyhow::Error>(records)
    });
    let mut input =
        InputConnection::connect_geometry(&input_base, expected, &[input_address]).await?;
    assert_eq!(input.display(), Some(display));
    assert!(!input.active());
    let first = Event::Move { x: 1024, y: 2048 };
    let second = Event::Move { x: 60000, y: 50000 };
    input.send(Event::Activate).await?;
    input.send(first).await?;
    generated_frame(&mut requests, &mut frames, config, 1, [3, 21, 90]).await?;
    assert!(input.active());

    if input_first {
        input.send(Event::Deactivate).await?;
        input.send(Event::End).await?;
        drop(input);
        assert_eq!(input_task.await??, 4);
        assert_eq!(
            consumer.await??,
            vec![Event::Activate, first, Event::Deactivate, Event::End]
        );
        input_base.close().await;
        input_host.close().await;
        // The independent view still receives newly generated pixels after
        // both input endpoints have closed, despite the identical peer IDs.
        generated_frame(&mut requests, &mut frames, config, 2, [91, 32, 17]).await?;
        let request = timeout(WAIT, requests.recv())
            .await?
            .context("Missing screen end request")?;
        ensure!(request.reply.send(Ok(Capture::End)).is_ok(), "End canceled");
        assert_eq!(video_task.await??.frames, 2);
        assert_eq!(view_task.await??.frames, 2);
        screen_base.close().await;
        screen_host.close().await;
    } else {
        let request = timeout(WAIT, requests.recv())
            .await?
            .context("Missing screen end request")?;
        ensure!(request.reply.send(Ok(Capture::End)).is_ok(), "End canceled");
        assert_eq!(video_task.await??.frames, 1);
        assert_eq!(view_task.await??.frames, 1);
        screen_base.close().await;
        screen_host.close().await;
        // Ending screen capture must not implicitly change input permission
        // or ownership; the separate local input stop/lease remains active.
        assert!(input.active());
        input.send(second).await?;
        input.send(Event::Deactivate).await?;
        input.send(Event::End).await?;
        drop(input);
        assert_eq!(input_task.await??, 5);
        assert_eq!(
            consumer.await??,
            vec![
                Event::Activate,
                first,
                second,
                Event::Deactivate,
                Event::End
            ]
        );
        input_base.close().await;
        input_host.close().await;
    }
    Ok(())
}

#[tokio::test]
async fn closing_input_does_not_close_same_identity_screen() -> Result<()> {
    timeout(Duration::from_secs(45), both(true)).await??;
    Ok(())
}

#[tokio::test]
async fn closing_screen_does_not_close_same_identity_input() -> Result<()> {
    timeout(Duration::from_secs(45), both(false)).await??;
    Ok(())
}
