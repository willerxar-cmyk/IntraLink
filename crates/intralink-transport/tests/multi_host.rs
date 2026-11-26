//! Exercise actual invitation grants, QUIC streams and Compact reconstruction
//! against three independent synthetic sources. No OS capture or remote input.
use std::{future::ready, net::SocketAddr, time::Duration};

use anyhow::{Context, Result, ensure};
use intralink_core::{
    Role,
    adaptive::ViewProtocol,
    desktop::{CodecMode, VideoConfig, ViewPriority},
    media::{Delta, MAX_DELTA_BYTES, apply_delta, apply_keyframe, encode_delta, encode_keyframe},
    profile_limits::ReceiverLimits,
};
use intralink_transport::{
    bind_workspace_controller,
    desktop::{
        Capture, CaptureRequest, Stats, ViewMode, bind_view_with_protocol, serve_view,
        serve_view_with_protocol, view_with_observations, view_with_policy,
    },
};
use intralink_vault::{Direction, Profile, Route};
use iroh::{Endpoint, EndpointId};
use tokio::{sync::mpsc, time::timeout};

const FRAMES: usize = 8;
const WAIT: Duration = Duration::from_secs(10);
const NOW: u64 = 1_700_000_000;

struct Host {
    endpoint: Endpoint,
    address: SocketAddr,
    approved: EndpointId,
    marker: u8,
    expected: EndpointId,
}

async fn paired_host(controller: &mut Profile, name: &str, marker: u8) -> Result<Host> {
    paired_host_protocol(controller, name, marker, ViewProtocol::Fixed).await
}

async fn paired_host_protocol(
    controller: &mut Profile,
    name: &str,
    marker: u8,
    protocol: ViewProtocol,
) -> Result<Host> {
    let mut host = Profile::new(Role::Host);
    let endpoint =
        bind_view_with_protocol(host.secret_key(), "127.0.0.1:0".parse()?, protocol).await?;
    let addr = endpoint
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("Missing loopback socket")?;
    let route = Route::new(&addr.ip().to_string(), addr.port())?;
    let offer = host.create_invite(controller.public_id(), route, NOW, 120)?;
    let acceptance = controller.accept_invite(name, &offer, NOW + 1)?;
    assert!(host.peer("main", Direction::Inbound).is_err());
    host.confirm_invite("main", &acceptance, NOW + 2)?;
    let grant = controller.peer(name, Direction::Outbound)?;
    let route = grant.route().context("Missing route in grant")?;
    let address = SocketAddr::new(route.host().parse()?, route.port());
    let expected = grant.identity()?;
    assert_eq!(expected, endpoint.id());
    assert_eq!(address, addr);
    // The host only knows its controller; it cannot enumerate sibling hosts.
    assert_eq!(host.peers().len(), 1);
    assert!(host.peer(name, Direction::Outbound).is_err());
    Ok(Host {
        endpoint,
        address,
        approved: host.peer("main", Direction::Inbound)?.identity()?,
        marker,
        expected,
    })
}

async fn run_session(controller: &Endpoint, host: &Host, priority: ViewPriority) -> Result<Stats> {
    let config = VideoConfig::new(540, 60, 75, 30)
        .context("Invalid profile")?
        .with_mode(CodecMode::Compact);
    run_profile_session(
        controller,
        host,
        priority,
        config,
        ViewMode::new(CodecMode::Compact, ViewProtocol::Fixed),
        config,
    )
    .await
}

async fn run_profile_session(
    controller: &Endpoint,
    host: &Host,
    priority: ViewPriority,
    approved: VideoConfig,
    mode: ViewMode,
    config: VideoConfig,
) -> Result<Stats> {
    let pixels = config.width() as usize * config.height() as usize;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let mut previous = [host.marker, 0, 0].repeat(pixels);
    let mut rgb = previous.clone();
    let producer = async {
        for sequence in 1..=FRAMES {
            let req = timeout(WAIT, requests.recv())
                .await?
                .context("Missing capture request")?;
            assert_eq!(req.desktop_config, Some(config));
            let mut payload = req.buffer;
            if sequence == 1 {
                ensure!(
                    encode_keyframe(config, &rgb, &mut payload, MAX_DELTA_BYTES)
                        .map_err(|err| anyhow::anyhow!("Keyframe: {err:?}"))?,
                    "Unexpected keyframe budget"
                );
            } else {
                let index = sequence * 3;
                rgb[index..index + 3].copy_from_slice(&[host.marker, sequence as u8, 11]);
                ensure!(
                    encode_delta(config, &previous, &rgb, &mut payload, MAX_DELTA_BYTES)
                        .map_err(|err| anyhow::anyhow!("Delta: {err:?}"))?
                        == Delta::Encoded,
                    "Missing delta"
                );
                previous[index..index + 3].copy_from_slice(&rgb[index..index + 3]);
            }
            let record = if mode.protocol.is_adaptive() {
                Capture::AdaptiveFrame {
                    height: config.height() as u16,
                    data: payload,
                }
            } else {
                Capture::Frame(payload)
            };
            ensure!(req.reply.send(Ok(record)).is_ok(), "Capture canceled");
        }
        let req = timeout(WAIT, requests.recv())
            .await?
            .context("Missing stop request")?;
        ensure!(req.reply.send(Ok(Capture::End)).is_ok(), "Capture canceled");
        Ok::<(), anyhow::Error>(())
    };
    let mut canvas = Vec::new();
    let mut decoded = 0usize;
    let view = view_with_observations(
        controller,
        host.expected,
        std::slice::from_ref(&host.address),
        mode,
        Vec::new,
        |frame| {
            let result = (|| -> Result<()> {
                decoded += 1;
                ensure!(
                    frame.sequence == decoded && frame.config == config,
                    "Unexpected sequence or profile"
                );
                if decoded == 1 {
                    apply_keyframe(config, &frame.payload, &mut canvas)
                        .map_err(|err| anyhow::anyhow!("Keyframe decode: {err:?}"))?;
                } else {
                    apply_delta(config, &frame.payload, &mut canvas)
                        .map_err(|err| anyhow::anyhow!("Delta decode: {err:?}"))?;
                }
                ensure!(canvas.len() == pixels, "Invalid canvas size");
                ensure!(
                    canvas[decoded]
                        == (u32::from(host.marker) << 16
                            | if decoded == 1 {
                                0
                            } else {
                                (decoded as u32) << 8 | 11
                            }),
                    "Wrong host or pixel data"
                );
                Ok(())
            })();
            ready(result)
        },
        |_| priority,
    );
    let (served, viewed, ()) = timeout(Duration::from_secs(40), async {
        tokio::try_join!(
            serve_view_with_protocol(
                &host.endpoint,
                host.approved,
                approved,
                source,
                mode.protocol
            ),
            view,
            producer
        )
    })
    .await??;
    assert_eq!(served, viewed);
    assert_eq!(viewed.frames, FRAMES);
    assert_eq!(viewed.repeats, 0);
    assert_eq!(decoded, FRAMES);
    assert!(viewed.media_bytes > 0);
    for (index, value) in canvas.into_iter().enumerate() {
        let extra = if (2..=FRAMES).contains(&index) {
            (index as u32) << 8 | 11
        } else {
            0
        };
        assert_eq!(value, u32::from(host.marker) << 16 | extra);
    }
    Ok(viewed)
}

#[tokio::test]
async fn one_controller_three_paired_hosts_keep_independent_compact_canvases() -> Result<()> {
    let mut profile = Profile::new(Role::Controller);
    let one = paired_host(&mut profile, "host-1", 17).await?;
    let two = paired_host(&mut profile, "host-2", 43).await?;
    let three = paired_host(&mut profile, "host-3", 91).await?;
    let controller = bind_workspace_controller(profile.secret_key()).await?;
    assert_eq!(controller.id(), profile.public_id());
    // Exercise both the foreground and background scheduling policies with one
    // endpoint. This is protocol validation, not a sustained-FPS benchmark.
    let results = tokio::join!(
        run_session(&controller, &one, ViewPriority::Foreground),
        run_session(&controller, &two, ViewPriority::Background),
        run_session(&controller, &three, ViewPriority::Foreground),
    );
    controller.close().await;
    tokio::join!(
        one.endpoint.close(),
        two.endpoint.close(),
        three.endpoint.close()
    );
    for result in [results.0, results.1, results.2] {
        assert_eq!(result?.frames, FRAMES);
    }
    Ok(())
}

/// Stop the local viewer after its first authenticated frame, while the host
/// keeps serving. Cancellation must terminate only the associated connection.
async fn cancel_session(controller: &Endpoint, host: &Host) -> Result<()> {
    let config = VideoConfig::new(540, 60, 75, 30)
        .context("Invalid profile")?
        .with_mode(CodecMode::Compact);
    let pixel_count = config.width() as usize * config.height() as usize;
    let rgb = [host.marker, 0, 0].repeat(pixel_count);
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        let mut first = true;
        while let Some(req) = requests.recv().await {
            let mut payload = req.buffer;
            payload.clear();
            let capture = if first {
                first = false;
                ensure!(
                    encode_keyframe(config, &rgb, &mut payload, MAX_DELTA_BYTES)
                        .map_err(|err| anyhow::anyhow!("Keyframe: {err:?}"))?,
                    "Unexpected keyframe budget"
                );
                Capture::Frame(payload)
            } else {
                Capture::Repeat(payload)
            };
            if req.reply.send(Ok(capture)).is_err() {
                break;
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    let (seen, mut observed) = tokio::sync::oneshot::channel();
    let mut seen = Some(seen);
    let client = async {
        let viewer = view_with_policy(
            controller,
            host.expected,
            std::slice::from_ref(&host.address),
            CodecMode::Compact,
            Vec::new,
            |frame| {
                let result = (|| -> Result<()> {
                    ensure!(
                        frame.sequence == 1 && frame.config == config,
                        "Unexpected first frame"
                    );
                    let mut canvas = Vec::new();
                    apply_keyframe(config, &frame.payload, &mut canvas)
                        .map_err(|err| anyhow::anyhow!("Keyframe decode: {err:?}"))?;
                    ensure!(
                        canvas.len() == pixel_count
                            && canvas
                                .iter()
                                .all(|&pixel| pixel == u32::from(host.marker) << 16),
                        "Wrong canceled-host canvas"
                    );
                    seen.take()
                        .context("Duplicate first frame")?
                        .send(())
                        .map_err(|_| anyhow::anyhow!("Observer closed"))?;
                    Ok(())
                })();
                ready(result)
            },
            || ViewPriority::Foreground,
        );
        tokio::pin!(viewer);
        tokio::select! {
            biased;
            result = &mut observed => result.context("No frame before cancellation"),
            result = &mut viewer => anyhow::bail!("Viewer ended before local cancellation: {result:?}"),
        }
        // Dropping `viewer` at this scope boundary must close just its connection.
    };
    let (served, viewed, produced) = timeout(Duration::from_secs(15), async {
        tokio::join!(
            serve_view(&host.endpoint, host.approved, config, source),
            client,
            producer
        )
    })
    .await
    .context("Canceled session did not release its connection and source")?;
    viewed?;
    produced?;
    ensure!(
        served.is_err(),
        "Canceled viewer was not reported as disconnected by host"
    );
    Ok(())
}

#[tokio::test]
async fn stopping_one_view_preserves_sibling_and_allows_later_connection() -> Result<()> {
    let mut profile = Profile::new(Role::Controller);
    let one = paired_host(&mut profile, "host-1", 21).await?;
    let two = paired_host(&mut profile, "host-2", 55).await?;
    let controller = bind_workspace_controller(profile.secret_key()).await?;
    assert_eq!(controller.id(), profile.public_id());
    let (stopped, continued) = tokio::join!(
        cancel_session(&controller, &one),
        run_session(&controller, &two, ViewPriority::Background),
    );
    // A later user-approved host can use the same still-open endpoint.
    let three = paired_host(&mut profile, "host-3", 98).await?;
    let later = run_session(&controller, &three, ViewPriority::Foreground).await;
    controller.close().await;
    tokio::join!(
        one.endpoint.close(),
        two.endpoint.close(),
        three.endpoint.close()
    );
    stopped?;
    assert_eq!(continued?.frames, FRAMES);
    assert_eq!(later?.frames, FRAMES);
    Ok(())
}

#[tokio::test]
async fn one_controller_three_paired_hosts_negotiate_independent_receiver_limits() -> Result<()> {
    let mut profile = Profile::new(Role::Controller);
    let protocol = ViewProtocol::Bounded;
    let one = paired_host_protocol(&mut profile, "host-1", 19, protocol).await?;
    let two = paired_host_protocol(&mut profile, "host-2", 47, protocol).await?;
    let three = paired_host_protocol(&mut profile, "host-3", 97, protocol).await?;
    let controller = bind_workspace_controller(profile.secret_key()).await?;
    let approved = VideoConfig::new(1080, 60, 75, 30)
        .unwrap()
        .with_mode(CodecMode::Compact);
    let two_approved = VideoConfig::new(720, 25, 75, 30)
        .unwrap()
        .with_mode(CodecMode::Compact);
    let one_mode = ViewMode::new(CodecMode::Compact, protocol);
    let two_mode = one_mode.with_limits(ReceiverLimits::new(720, 30).unwrap());
    let three_mode = one_mode.with_limits(ReceiverLimits::new(540, 15).unwrap());
    let three_config = VideoConfig::new(540, 15, 75, 30)
        .unwrap()
        .with_mode(CodecMode::Compact);
    let results = tokio::join!(
        run_profile_session(
            &controller,
            &one,
            ViewPriority::Foreground,
            approved,
            one_mode,
            approved
        ),
        run_profile_session(
            &controller,
            &two,
            ViewPriority::Foreground,
            two_approved,
            two_mode,
            two_approved
        ),
        run_profile_session(
            &controller,
            &three,
            ViewPriority::Background,
            approved,
            three_mode,
            three_config
        ),
    );
    controller.close().await;
    tokio::join!(
        one.endpoint.close(),
        two.endpoint.close(),
        three.endpoint.close()
    );
    for result in [results.0, results.1, results.2] {
        assert_eq!(result?.frames, FRAMES);
    }
    Ok(())
}
