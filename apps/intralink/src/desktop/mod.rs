//! Experimental Windows/X11 screen viewing. Local approval is session-specific.
mod calibration;
mod capture_fixture;
pub use capture_fixture::PatternArgs;
mod codec;
mod host_calibration;
mod jpeg;
mod jpeg_benchmark;
pub use calibration::ProfileCheckArgs;
#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
pub mod decoder_ipc;
#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod decoder_process;
#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
pub mod h264_check;
mod host_capture;
#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod live_h264;
#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod precise_timer;
pub use host_capture::CaptureBackend;
use host_capture::{HostCapture, HostDisplay};
#[cfg(target_os = "windows")]
pub mod media_check;
mod overview;
mod receive_limits;
mod resize;
pub use receive_limits::ReceiveProfileArgs;
mod source_status;
#[cfg(all(target_os = "windows", feature = "wgc-preview"))]
mod wgc_check;
pub mod workspace;

use std::{
    io::{self, IsTerminal},
    net::SocketAddr,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, ensure};
use intralink_core::{
    adaptive::{AdaptiveResolution, ViewProtocol},
    desktop::{VideoConfig, ViewPriority},
    journal::Outcome,
    media::{JPEG_TAG, LOSSLESS_KEYFRAME_TAG, TILE_TAG},
    quality::AdaptiveQuality,
};
use intralink_transport::{
    bind_controller,
    desktop::{
        Capture, CaptureRequest, Stats, VideoFrame, ViewMode, bind_view_with_protocol,
        serve_view_with_protocol, view_with_observations,
    },
};
use intralink_vault::{Direction, Vault};
use minifb::{Key, ScaleMode, Window, WindowOptions};
use tokio::sync::{mpsc, oneshot};
use xcap::Monitor;

use crate::{
    local::{approve, prompt_password},
    routing,
};

fn require_graphical_terminal() -> Result<()> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "A local interactive graphical terminal is required"
    );
    require_graphical_environment()
}

// The local-only rendering diagnostic needs a graphical environment, not
// an interactive consent terminal. Actual sharing still requires both.
fn require_graphical_environment() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        ensure!(
            std::env::var_os("DISPLAY").is_some(),
            "An active X11 desktop is required"
        );
        ensure!(
            std::env::var_os("WAYLAND_DISPLAY").is_none()
                && std::env::var("XDG_SESSION_TYPE").ok().as_deref() != Some("wayland"),
            "Wayland capture is not enabled in this preview; a permissioned portal adapter is required"
        );
    }
    Ok(())
}

fn monitors() -> Result<Vec<Monitor>> {
    Monitor::all().map_err(|err| anyhow!("Cannot enumerate local displays: {err}"))
}

pub fn list_displays() -> Result<()> {
    require_graphical_terminal()?;
    for monitor in monitors()? {
        let id = monitor
            .id()
            .map_err(|err| anyhow!("Display ID unavailable: {err}"))?;
        let name = monitor
            .name()
            .map_err(|err| anyhow!("Display name unavailable: {err}"))?;
        let width = monitor
            .width()
            .map_err(|err| anyhow!("Display width unavailable: {err}"))?;
        let height = monitor
            .height()
            .map_err(|err| anyhow!("Display height unavailable: {err}"))?;
        println!("{id}: {name:?} ({width} x {height})");
    }
    Ok(())
}

pub fn view_protocol(adaptive: bool, negotiated: bool, bounded: bool) -> ViewProtocol {
    if bounded {
        ViewProtocol::Bounded
    } else if negotiated {
        ViewProtocol::Negotiated
    } else if adaptive {
        ViewProtocol::Adaptive
    } else {
        ViewProtocol::Fixed
    }
}

#[derive(Clone, Copy)]
pub struct StreamOptions {
    pub fixed_quality: bool,
    pub protocol: ViewProtocol,
}

/// Network tasks never own a native GUI or capture handle. Cancellation has
/// priority over incoming data; the foreground window keeps processing events.
pub async fn share(
    path: &Path,
    controller: &str,
    local: SocketAddr,
    monitor_id: u32,
    config: VideoConfig,
    backend: CaptureBackend,
    options: StreamOptions,
) -> Result<Outcome> {
    let StreamOptions {
        fixed_quality,
        protocol,
    } = options;
    require_graphical_terminal()?;
    backend.validate()?;
    ensure!(local.port() != 0, "Specify a nonzero listening port");
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    drop(password);
    ensure!(
        profile.role().can_receive(),
        "This profile cannot receive sessions"
    );
    let peer = profile.peer(controller, Direction::Inbound)?.identity()?;
    let secret = profile.secret_key();
    drop(profile);
    let display = HostDisplay::select(monitor_id)?;
    let (backend, config) = if backend == CaptureBackend::Auto {
        // Only an owned generated window is read before approval. Resolve a
        // concrete measured choice, then verify the selected real display again.
        let measured = host_calibration::select(config)?;
        display.verify()?;
        (
            CaptureBackend::from_measured(measured.path),
            measured.config,
        )
    } else {
        (backend, config)
    };
    let name = display.name();
    let (width, height) = display.size();
    println!(
        "Controller: {controller} ({peer})\nDisplay: {name:?} ({monitor_id}, {width} x {height})\nOutput: {} x {}, max {} fps, mode {:?}, JPEG quality {}, duration {}s.",
        config.width(),
        config.height(),
        config.fps(),
        config.mode(),
        config.quality(),
        config.seconds()
    );
    println!("Explicit capture backend: {backend:?}");
    if matches!(protocol, ViewProtocol::Negotiated | ViewProtocol::Bounded) {
        let caps = intralink_core::capabilities::CodecCapabilities::through(config.mode());
        println!(
            "Approved software codec ceiling: {:?}; allowed mask: {:#05b}. Best mutual codec will be selected before capture.",
            config.mode(),
            caps.bits()
        );
    }
    if fixed_quality {
        println!("JPEG quality: fixed at {}", config.quality());
    } else {
        println!(
            "JPEG quality: adaptive, 35-{} (receipt pressure)",
            config.quality()
        );
    }
    println!(
        "Resolution policy: {protocol:?}; approved maximum {}p; adaptive minimum 540p",
        config.height()
    );
    approve(
        "Share the entire selected display with this exact controller for ONE read-only session? Close sensitive windows first. The recipient may save what they see. Ctrl+C stops the foreground process.",
    )?;
    // Open and paint the stop window before the listener. Do not capture if the
    // indicator cannot be displayed. It remains local throughout the session.
    let mut indicator = Window::new(
        "IntraLink: Waiting - ESC to cancel",
        320,
        64,
        WindowOptions {
            topmost: true,
            ..WindowOptions::default()
        },
    )
    .map_err(|err| anyhow!("Cannot show local sharing indicator: {err}"))?;
    // Event polling is independent of media pacing; do not sleep twice per frame.
    indicator.set_target_fps(0);
    let waiting = vec![0x00ad7100; 320 * 64];
    let sharing = vec![0x00c52d3b; 320 * 64];
    indicator
        .update_with_buffer(&waiting, 320, 64)
        .map_err(|err| anyhow!("Sharing indicator failed: {err}"))?;
    let endpoint = bind_view_with_protocol(secret, local, protocol).await?;
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let (stop, mut stop_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let result = tokio::select! {
            biased;
            _ = &mut stop_rx => Ok(None),
            outcome = serve_view_with_protocol(&endpoint, peer, config, source, protocol) => outcome.map(Some),
        };
        endpoint.close().await;
        result
    });
    println!("Waiting at {local} for the approved peer (up to 120s). No capture has started.");
    let mut encoder = None::<codec::Encoder>;
    let mut quality = None::<AdaptiveQuality>;
    let mut resolution = None::<AdaptiveResolution>;
    let mut selected_config = None::<VideoConfig>;
    let mut current_height = config.height() as u16;
    let mut active = false;
    let mut capture = None::<HostCapture>;
    let mut last_health_check = Instant::now();
    let mut last_indicator_paint = Instant::now();
    let mut capture_time = Duration::ZERO;
    let mut encode_time = Duration::ZERO;
    let mut sampled = 0u64;
    let mut keyframes = 0u64;
    let mut lossless = 0u64;
    let mut tiles = 0u64;
    let started = Instant::now();
    let local_result = async {
        while !task.is_finished() && indicator.is_open() && !indicator.is_key_down(Key::Escape) {
            if last_indicator_paint.elapsed() >= Duration::from_millis(100) {
                let banner = if active { &sharing } else { &waiting };
                indicator
                    .update_with_buffer(banner, 320, 64)
                    .map_err(|err| anyhow!("Sharing indicator failed: {err}"))?;
                last_indicator_paint = Instant::now();
            } else {
                indicator.update();
            }
            if !indicator.is_open() || indicator.is_key_down(Key::Escape) {
                break;
            }
            if last_health_check.elapsed() >= Duration::from_millis(250) {
                if let Some(source) = &capture {
                    display.verify()?;
                    source.check_health()?;
                }
                last_health_check = Instant::now();
            }
            // Wake immediately for capture demand; otherwise pump the native
            // stop window on a bounded timer instead of polling every 1 ms.
            let request = match tokio::time::timeout(Duration::from_millis(8), requests.recv()).await {
                Ok(Some(request)) => request,
                Ok(None) => break,
                Err(_) => continue,
            };
            indicator.update();
            if !indicator.is_open() || indicator.is_key_down(Key::Escape) {
                break;
            }
            if request.reply.is_closed() {
                break;
            }
            if !active {
                indicator.set_title("IntraLink: SCREEN SHARING - ESC to stop");
                indicator
                    .update_with_buffer(&sharing, 320, 64)
                    .map_err(|err| anyhow!("Sharing indicator failed: {err}"))?;
                active = true;
                last_indicator_paint = Instant::now();
                println!(
                    "SCREEN SHARING ACTIVE. Focus the red window and press ESC, close it, or press Ctrl+C."
                );
                if !indicator.is_open() || indicator.is_key_down(Key::Escape) {
                    break;
                }
            }
            // Capture construction is reachable only after the transport has
            // authenticated the approved peer and matched the view request.
            let selected = request.desktop_config.context("Missing authenticated desktop profile")?;
            ensure!(
                if protocol == ViewProtocol::Bounded {
                    intralink_core::profile_limits::ReceiverLimits::within_approval(config, selected)
                } else {
                    selected == config.with_mode(selected.mode())
                },
                "Negotiation exceeded the approved media profile"
            );
            ensure!(
                if matches!(protocol, ViewProtocol::Negotiated | ViewProtocol::Bounded) {
                    intralink_core::capabilities::CodecCapabilities::through(config.mode()).contains(selected.mode())
                } else {
                    selected.mode() == config.mode()
                },
                "Selected codec exceeds local approval"
            );
            if let Some(previous) = selected_config {
                ensure!(previous == selected, "Media profile changed after negotiation");
            } else {
                // Allocate and initialize only after the authenticated selection.
                display.verify()?;
                encoder = Some(codec::Encoder::new(selected));
                quality = (!fixed_quality).then(|| AdaptiveQuality::new(selected));
                resolution = protocol.is_adaptive().then(|| AdaptiveResolution::new(selected));
                capture = Some(HostCapture::start(&display, backend, selected)?);
                selected_config = Some(selected);
                current_height = selected.height() as u16;
                println!("Selected software codec: {:?}, {}p, max {} FPS", selected.mode(), current_height, selected.fps());
                indicator.set_title(&format!("IntraLink: SCREEN SHARING - {:?} - {current_height}p - ESC to stop", selected.mode()));
            }
            let encoder = encoder.as_mut().context("Encoder missing")?;
            let source = capture.as_mut().context("Capture source missing")?;
            let capture_started = Instant::now();
            let observed = source.capture(&display, || {
                indicator.update();
                ensure!(indicator.is_open() && !indicator.is_key_down(Key::Escape)
                    && !request.reply.is_closed() && !task.is_finished(), "Local sharing stopped");
                Ok(())
            });
            capture_time += capture_started.elapsed();
            if !indicator.is_open() || indicator.is_key_down(Key::Escape)
                || request.reply.is_closed() || task.is_finished() {
                break;
            }
            let receipt_pressure = request.receipt_pressure;
            let mut local_encode_time = Duration::ZERO;
            let encoded = match observed? {
                Some(frame) => {
                    let encode_started = Instant::now();
                    let encoded = encoder.encode(&frame, request.buffer)?;
                    local_encode_time = encode_started.elapsed();
                    encode_time += local_encode_time;
                    sampled += 1;
                    source.recycle(frame);
                    encoded
                }
                None => Capture::NoUpdate(request.buffer),
            };
            display.verify()?;
            source.check_health()?;
            if let Capture::Frame(data) = &encoded {
                match data.first().copied() {
                    Some(JPEG_TAG) => keyframes += 1,
                    Some(LOSSLESS_KEYFRAME_TAG) => lossless += 1,
                    Some(TILE_TAG) => tiles += 1,
                    _ => anyhow::bail!("Unknown locally encoded media record"),
                }
            }
            // Recheck local stop after synchronous OS capture/encoding, before
            // handing pixels to the independent transport task.
            indicator.update();
            if !indicator.is_open() || indicator.is_key_down(Key::Escape) {
                break;
            }
            let changed = matches!(&encoded, Capture::Frame(_));
            let encoded = match encoded {
                Capture::Frame(data) if protocol.is_adaptive() => Capture::AdaptiveFrame { height: current_height, data },
                other => other,
            };
            if request.reply.send(Ok(encoded)).is_err() {
                break;
            }
            if let Some(policy) = quality.as_mut()
                && let Some(next) = policy.observe(receipt_pressure, changed)
            {
                encoder.set_jpeg_quality(next)?;
                println!("Adaptive JPEG target: {next} (receipt pressure: {receipt_pressure:?}); next JPEG keyframe");
                indicator.set_title(&format!("IntraLink: SCREEN SHARING - {current_height}p - JPEG target {next} - ESC to stop"));
            }
            if let Some(policy) = resolution.as_mut()
                && let Some(next) = policy.observe(receipt_pressure, local_encode_time, changed)
            {
                source.retarget(next)?;
                encoder.reconfigure(next)?;
                current_height = next.height() as u16;
                println!("Adaptive resolution target: {current_height}p (next complete keyframe)");
                indicator.set_title(&format!("IntraLink: SCREEN SHARING - {current_height}p - ESC to stop"));
            }
        }
        Ok::<(), anyhow::Error>(())
    }.await;
    // Always signal cancellation before joining, including UI error paths.
    // A native capture call cannot itself be preempted by the window loop.
    if !task.is_finished() {
        let _ = stop.send(());
    }
    let capture_result = capture.as_mut().map_or(Ok(()), HostCapture::stop);
    if let Err(error) = &capture_result {
        eprintln!("Native capture shutdown: {error:#}");
    }
    let network_result = task.await.context("Desktop network task failed")?;
    drop(indicator);
    drop(vault);
    println!("Sharing stopped; listener closed.");
    if sampled > 0 {
        println!(
            "Local capture+encode samples: {sampled}; JPEG keyframes: {keyframes}; lossless keyframes: {lossless}; tile updates: {tiles}; total capture {:.3} ms; total resize/compare/encode {:.3} ms",
            capture_time.as_secs_f64() * 1000.0,
            encode_time.as_secs_f64() * 1000.0
        );
    }
    local_result?;
    capture_result?;
    Ok(report(network_result?, started))
}

pub async fn receive(path: &Path, device: &str, mode: ViewMode) -> Result<Outcome> {
    require_graphical_terminal()?;
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    drop(password);
    ensure!(
        profile.role().can_initiate(),
        "This profile cannot initiate sessions"
    );
    let peer = profile.peer(device, Direction::Outbound)?;
    let identity = peer.identity()?;
    let route = peer.route().context("Outbound route missing")?.clone();
    let secret = profile.secret_key();
    drop(profile);
    println!("Request one view-only session from {device} ({identity}) in {mode:?} mode.");
    let candidates = routing::resolve(&route).await?;
    let mut window = Window::new(
        "IntraLink: Connecting - ESC to disconnect",
        960,
        540,
        WindowOptions {
            resize: true,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .map_err(|err| anyhow!("Cannot open the remote view: {err}"))?;
    window.set_target_fps(60);
    let endpoint = bind_controller(secret, &candidates).await?;
    // Stateful updates must arrive in order. One queued changed record is
    // sufficient to absorb short rendering delays; backpressure replaces drops.
    let (frames, mut pending) = mpsc::channel::<VideoFrame>(1);
    // Return only consumed payload allocations. A full pool discards the
    // allocation rather than blocking the window or increasing queue size.
    let (recycle, mut recycled) = mpsc::channel::<Vec<u8>>(2);
    let source_status = Arc::new(source_status::SourceStatus::default());
    let observed = source_status.clone();
    let (stop, mut stop_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let result = tokio::select! {
            biased;
            _ = &mut stop_rx => Ok(None),
            outcome = view_with_observations(&endpoint, identity, &candidates, mode,
                || recycled.try_recv().unwrap_or_default(),
                |frame| {
                    let frames = frames.clone();
                    async move {
                        frames.send(frame).await.map_err(|_| anyhow!("Local viewer stopped"))
                    }
                },
                move |record| { observed.observe(record); ViewPriority::Foreground },
            ) => outcome.map(Some),
        };
        endpoint.close().await;
        result
    });
    let mut pixels = vec![0; 960 * 540];
    let mut dimensions = (960, 540);
    let mut last_sequence = 0;
    let mut notice = false;
    let mut banner_sequence = 0;
    let mut banner_pixels = Vec::new();
    let mut decoder = codec::Decoder::default();
    let mut rendered = 0u64;
    let mut keyframes = 0u64;
    let mut lossless = 0u64;
    let mut tiles = 0u64;
    let mut decode_time = Duration::ZERO;
    let started = Instant::now();
    let local_result = (|| -> Result<()> {
        while window.is_open() && !window.is_key_down(Key::Escape) {
            if let Ok(frame) = pending.try_recv() {
                ensure!(frame.sequence > last_sequence, "Out-of-order media record");
                let decode_started = Instant::now();
                decoder.decode_into_protocol(
                    frame.config,
                    &frame.payload,
                    &mut pixels,
                    mode.protocol,
                )?;
                decode_time += decode_started.elapsed();
                rendered += 1;
                match frame.payload.first().copied() {
                    Some(JPEG_TAG) => keyframes += 1,
                    Some(LOSSLESS_KEYFRAME_TAG) => lossless += 1,
                    Some(TILE_TAG) => tiles += 1,
                    _ => anyhow::bail!("Unknown decoded media record"),
                }
                let next_dimensions = (
                    frame.config.width() as usize,
                    frame.config.height() as usize,
                );
                if (last_sequence == 0 || dimensions != next_dimensions) && !notice {
                    window.set_title(&format!(
                        "IntraLink: Remote view - {}p - ESC to disconnect",
                        frame.config.height()
                    ));
                }
                dimensions = next_dimensions;
                last_sequence = frame.sequence;
                let mut buffer = frame.payload;
                buffer.clear();
                // No future decoder access to these bytes; the pixel canvas is independent.
                let _ = recycle.try_send(buffer);
            }
            let next_notice = last_sequence != 0 && source_status.notice_due();
            if next_notice && (!notice || banner_sequence != last_sequence) {
                overview::last_frame_banner(&pixels, dimensions, &mut banner_pixels)?;
                banner_sequence = last_sequence;
            }
            if next_notice != notice {
                if next_notice {
                    window.set_title(
                        "IntraLink: LAST FRAME - no new native sample - ESC to disconnect",
                    );
                } else {
                    window.set_title(&format!(
                        "IntraLink: Remote view - {}p - ESC to disconnect",
                        dimensions.1
                    ));
                }
                notice = next_notice;
            }
            let visible = if notice { &banner_pixels } else { &pixels };
            window
                .update_with_buffer(visible, dimensions.0, dimensions.1)
                .map_err(|err| anyhow!("Viewer update failed: {err}"))?;
            if task.is_finished() && pending.is_empty() {
                break;
            }
        }
        Ok(())
    })();
    if !task.is_finished() {
        let _ = stop.send(());
    }
    drop(pending);
    drop(recycle);
    let network_result = task.await.context("Desktop network task failed")?;
    drop(window);
    drop(vault);
    println!(
        "Viewer disconnected. Decoded changed frames: {rendered}; JPEG keyframes: {keyframes}; lossless keyframes: {lossless}; tile updates: {tiles}; total decode {:.3} ms",
        decode_time.as_secs_f64() * 1000.0
    );
    local_result?;
    Ok(report(network_result?, started))
}

fn report(stats: Option<Stats>, started: Instant) -> Outcome {
    if let Some(stats) = stats {
        println!(
            "Changed frames: {}; unchanged records: {}; no-update records: {}; Media payload: {} bytes; total foreground elapsed: {:.2}s",
            stats.frames,
            stats.repeats,
            stats.no_updates,
            stats.media_bytes,
            started.elapsed().as_secs_f64()
        );
        Outcome::Completed
    } else {
        println!("Session cancelled locally.");
        Outcome::Cancelled
    }
}
