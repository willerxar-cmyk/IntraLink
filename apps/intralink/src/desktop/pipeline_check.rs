//! Windows local-only capture/encode/QUIC comparison. The only source is
//! an owned generated child window. No vault, real monitor or remote device.
use anyhow::{Context, Result, ensure};
use intralink_core::desktop::{CodecMode, VideoConfig};
use intralink_transport::{
    bind,
    desktop::{Capture, CaptureRequest, Stats, bind_view, serve_view, serve_view_pipelined, view},
};
use iroh::SecretKey;
use std::{
    future::ready,
    net::SocketAddr,
    process::Command,
    time::{Duration, Instant},
};
use tokio::{sync::mpsc, task::JoinHandle};

use super::{
    capture_fixture::{Fixture, TITLE, find_fixture},
    codec::Decoder,
    host_calibration::verify_marker,
    host_pipeline::EncodeLane,
    wgc_check::CaptureSession,
};

const SAMPLES: usize = 60;
const LIMIT: Duration = Duration::from_secs(18);

#[derive(Default)]
struct Observations {
    submitted: usize,
    completed: usize,
    native: usize,
    overlapping: usize,
    capture: Duration,
    encode: Duration,
    elapsed: Duration,
}

fn check_source(child: &mut Fixture, target: &xcap::Window, started: Instant) -> Result<()> {
    ensure!(
        started.elapsed() < LIMIT,
        "Local pipeline diagnostic deadline"
    );
    ensure!(
        child.0.try_wait()?.is_none()
            && target.pid().ok() == Some(child.0.id())
            && target.title().ok().as_deref() == Some(TITLE)
            && target.is_minimized().ok() == Some(false),
        "Owned diagnostic window changed or closed"
    );
    Ok(())
}

async fn produce(
    config: VideoConfig,
    child: &mut Fixture,
    target: &xcap::Window,
    mut requests: mpsc::Receiver<CaptureRequest>,
    server: &JoinHandle<Result<Stats>>,
) -> Result<Observations> {
    let started = Instant::now();
    let pid = child.0.id();
    let mut session = None::<CaptureSession>;
    let mut lane = EncodeLane::new(config);
    let mut stats = Observations::default();
    let mut first_demand = None::<Instant>;
    let mut next_capture = Instant::now();
    let interval = Duration::from_secs_f64(1.0 / f64::from(config.fps()));
    let result = async {
        loop {
            check_source(child, target, started)?;
            if server.is_finished() { break; }
            let busy = lane.busy();
            let available = lane.can_accept() && (stats.submitted < SAMPLES || !busy);
            let remaining = next_capture.saturating_duration_since(Instant::now());
            let poll = if available && !remaining.is_zero() { remaining.min(Duration::from_millis(8)) } else { Duration::from_millis(8) };
            tokio::select! {
                biased;
                done = lane.complete(), if busy => {
                    let done = done?;
                    let source = session.as_mut().context("Missing owned capture session")?;
                    if let Some(frame) = done.frame { source.recycle(frame); }
                    check_source(child,target,started)?;
                    source.check_health()?;
                    ensure!(!done.reply.is_closed(), "Diagnostic response canceled");
                    ensure!(done.reply.send(Ok(done.record?)).is_ok(), "Diagnostic response dropped");
                    stats.completed += 1;
                    stats.encode += done.elapsed;
                    lane.advance()?;
                }
                request = requests.recv(), if available && remaining.is_zero() => {
                    let Some(request) = request else { break; };
                    ensure!(!request.reply.is_closed() && request.desktop_config == Some(config), "Invalid local diagnostic demand");
                    if stats.submitted == SAMPLES {
                        ensure!(stats.completed == SAMPLES && !lane.busy(), "Unfinished diagnostic encoder");
                        ensure!(request.reply.send(Ok(Capture::End)).is_ok(), "Diagnostic end dropped");
                        stats.elapsed = first_demand.context("No diagnostic demand")?.elapsed();
                        break;
                    }
                    if session.is_none() {
                        first_demand = Some(Instant::now());
                        session = Some(CaptureSession::start_fixture(pid,TITLE,Some((config.width(),config.height())))?);
                    }
                    let source = session.as_mut().context("Missing owned capture session")?;
                    let overlapping = lane.encoding();
                    let began = Instant::now();
                    next_capture = began + interval;
                    let frame = source.capture(|| {
                        check_source(child,target,started)?;
                        ensure!(!request.reply.is_closed() && !server.is_finished(), "Diagnostic demand canceled");
                        Ok(())
                    })?;
                    stats.capture += began.elapsed();
                    check_source(child,target,started)?;
                    source.check_health()?;
                    if let Some((ref image, _)) = frame {
                        let p = image.get_pixel(image.width()/2,image.height()/2);
                        ensure!(p[0].abs_diff(70)<=3 && p[1].abs_diff(120)<=3 && p[2].abs_diff(180)<=3, "Owned dense pattern marker mismatch");
                        stats.native += 1;
                        stats.overlapping += usize::from(overlapping);
                    }
                    lane.submit(frame.map(|(image,_)|image),request)?;
                    stats.submitted += 1;
                }
                _ = tokio::time::sleep(poll) => {}
            }
        }
        ensure!(stats.submitted == SAMPLES && stats.completed == SAMPLES && !stats.elapsed.is_zero(), "Incomplete local diagnostic run");
        Ok::<(),anyhow::Error>(())
    }.await;
    // Drop all pending demand handles and join CPU/native work on every exit.
    let native = session.as_mut().map_or(Ok(()), CaptureSession::stop);
    let joined = lane.stop().await;
    result?;
    native?;
    joined?;
    Ok(stats)
}

async fn run_one(overlap: bool) -> Result<()> {
    let config = VideoConfig::new(1080, 60, 75, 12)
        .unwrap()
        .with_mode(CodecMode::Compact);
    let mut child = Fixture(
        Command::new(std::env::current_exe()?)
            .args([
                "desktop-test-pattern",
                "--height",
                "1080",
                "--dense-pattern",
            ])
            .spawn()
            .context("Start owned dense animation")?,
    );
    let target = find_fixture(&mut child.0)?;
    let host = bind_view(SecretKey::generate(), "127.0.0.1:0".parse()?).await?;
    let controller = bind(SecretKey::generate(), "127.0.0.1:0".parse()?, false).await?;
    let address = host
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .context("Missing loopback address")?;
    ensure!(
        address.ip().is_loopback(),
        "Diagnostic must remain loopback"
    );
    let host_id = host.id();
    let approved = controller.id();
    let (source, requests) = mpsc::channel(1);
    let host_endpoint = host.clone();
    let server = tokio::spawn(async move {
        if overlap {
            serve_view_pipelined(&host_endpoint, approved, config, source).await
        } else {
            serve_view(&host_endpoint, approved, config, source).await
        }
    });
    let viewer_endpoint = controller.clone();
    let viewer = tokio::spawn(async move {
        let mut decoder = Decoder::default();
        let mut pixels = Vec::new();
        let mut previous = 0;
        view(
            &viewer_endpoint,
            host_id,
            std::slice::from_ref(&address),
            config.mode(),
            |frame| {
                ready((|| {
                    ensure!(
                        frame.config == config && frame.sequence > previous,
                        "Diagnostic media sequence/geometry mismatch"
                    );
                    previous = frame.sequence;
                    decoder.decode_into(config, &frame.payload, &mut pixels)?;
                    verify_marker(config, &pixels)
                })())
            },
        )
        .await
    });
    let observations = produce(config, &mut child, &target, requests, &server).await;
    if observations.is_err() {
        server.abort();
        viewer.abort();
    }
    let (served, viewed) = tokio::join!(server, viewer);
    host.close().await;
    controller.close().await;
    let local = observations?;
    let served = served??;
    let viewed = viewed??;
    ensure!(
        served == viewed && served.frames + served.repeats + served.no_updates == SAMPLES,
        "Local pipeline transfer/receipt counts differ"
    );
    ensure!(
        served.frames > 1 && local.native > 1,
        "No changing native frames"
    );
    ensure!(
        overlap || local.overlapping == 0,
        "Serial diagnostic overlapped capture"
    );
    println!(
        "pipeline={overlap}, records={SAMPLES}, changed={}, repeats={}, no_updates={}, native_samples={}, overlapping_captures={}, elapsed_s={:.3}, records_fps={:.2}, changed_fps={:.2}, capture_ms={:.3}, encode_ms={:.3}, media_bytes={}",
        served.frames,
        served.repeats,
        served.no_updates,
        local.native,
        local.overlapping,
        local.elapsed.as_secs_f64(),
        SAMPLES as f64 / local.elapsed.as_secs_f64(),
        served.frames as f64 / local.elapsed.as_secs_f64(),
        local.capture.as_secs_f64() * 1000.0,
        local.encode.as_secs_f64() * 1000.0,
        served.media_bytes
    );
    Ok(())
}

pub(super) async fn run() -> Result<()> {
    super::require_graphical_environment()?;
    run_one(false).await?;
    run_one(true).await?;
    println!("pipeline_check=passed; generated owned window and authenticated QUIC loopback only.");
    Ok(())
}
