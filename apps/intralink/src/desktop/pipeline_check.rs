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
    task::Poll,
    time::{Duration, Instant},
};
use tokio::{sync::mpsc, task::JoinHandle};

use super::{
    capture_fixture::{Fixture, TITLE, find_fixture},
    codec::Decoder,
    host_pipeline::{DemandGate, EncodeLane},
    resize::CanvasResizer,
    visual_fidelity::{Fidelity, Reference},
    visual_fixture,
    wgc_check::{CaptureSession, CaptureTiming},
};

const SAMPLES: usize = 60;
const STAGED_SAMPLES: usize = 180;
const LIMIT: Duration = Duration::from_secs(18);
const STAGED_POLL: Duration = Duration::from_millis(1);

#[derive(Clone, Copy)]
struct CapturePolicy {
    bounded_lock_wait: bool,
    staged_readback: bool,
    samples: usize,
}

struct CaptureAttempt {
    request: CaptureRequest,
    began: Instant,
    overlapping: bool,
    next_poll: Instant,
}

#[derive(Default)]
struct Observations {
    submitted: usize,
    completed: usize,
    native: usize,
    overlapping: usize,
    capture: Duration,
    encode: Duration,
    elapsed: Duration,
    planned: Vec<Duration>,
    late: Vec<Duration>,
    cadence: Vec<Duration>,
    native_times: Vec<Duration>,
    capture_breakdown: Vec<CaptureTiming>,
    queue: Vec<Duration>,
    encode_times: Vec<Duration>,
    publication: Vec<Duration>,
}

impl Observations {
    fn submit_capture(
        &mut self,
        attempt: CaptureAttempt,
        frame: Option<super::wgc_check::Captured>,
        breakdown: CaptureTiming,
        lane: &mut EncodeLane,
    ) -> Result<()> {
        ensure!(
            frame.is_some() == breakdown.readback.is_some(),
            "Native timing/readback mismatch"
        );
        let elapsed = attempt.began.elapsed();
        self.capture += elapsed;
        self.native_times.push(elapsed);
        self.capture_breakdown.push(breakdown);
        if let Some((ref image, _)) = frame {
            visual_fixture::verify_native(image)?;
            self.native += 1;
            self.overlapping += usize::from(attempt.overlapping);
        }
        lane.submit(frame.map(|(image, _)| image), attempt.request)?;
        self.submitted += 1;
        Ok(())
    }
}

struct DeliveryTrace {
    sequence: usize,
    published_at: Instant,
}

struct TraceChannels {
    references: Option<mpsc::Sender<Reference>>,
    traces: mpsc::Sender<DeliveryTrace>,
}

#[derive(Default)]
struct ViewerTiming {
    delivery: Vec<Duration>,
    decode: Vec<Duration>,
}

fn timing(label: &str, values: &[Duration]) {
    if values.is_empty() {
        return;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let ms = |rank: usize| sorted[(sorted.len() * rank).div_ceil(100) - 1].as_secs_f64() * 1000.0;
    println!(
        "stage={label}, samples={}, p50_ms={:.3}, p95_ms={:.3}, p99_ms={:.3}, worst_ms={:.3}",
        sorted.len(),
        ms(50),
        ms(95),
        ms(99),
        ms(100)
    );
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
    channels: TraceChannels,
    policy: CapturePolicy,
) -> Result<Observations> {
    let samples = policy.samples;
    let TraceChannels { references, traces } = channels;
    let started = Instant::now();
    let pid = child.0.id();
    let mut session = None::<CaptureSession>;
    let mut lane = EncodeLane::new(config);
    let mut reference_resize = CanvasResizer::default();
    let mut stats = Observations::default();
    let mut first_demand = None::<Instant>;
    let mut gate = DemandGate::new(config);
    let mut last_capture = None::<Instant>;
    let mut pending = None::<CaptureAttempt>;
    let result = async {
        loop {
            check_source(child, target, started)?;
            if server.is_finished() { break; }
            let busy = lane.busy();
            let available = pending.is_none() && lane.can_accept() && (stats.submitted < samples || !busy);
            let due = gate.due();
            let staged_due = pending.as_ref().map(|attempt| attempt.next_poll);
            tokio::select! {
                biased;
                done = lane.complete(), if busy => {
                    let done = done?;
                    let source = session.as_mut().context("Missing owned capture session")?;
                    check_source(child,target,started)?;
                    source.check_health()?;
                    ensure!(!done.reply.is_closed(), "Diagnostic response canceled");
                    let record = done.record?;
                    if let Some(sender) = &references
                        && let Capture::Frame(data) = &record
                    {
                        let image = done.frame.as_ref().context("Missing captured reference")?;
                        let size = (config.width(), config.height());
                        let mut rgb = vec![0; (size.0 * size.1 * 3) as usize];
                        reference_resize.fill_rgb(image, &mut rgb, size)?;
                        sender.try_send(Reference {
                            sequence: stats.completed + 1,
                            tag: data[0],
                            rgb,
                        }).context("Visual reference window exceeded")?;
                    }
                    if let Some(frame) = done.frame { source.recycle(frame); }
                    let published_at = Instant::now();
                    if matches!(record, Capture::Frame(_)) {
                        traces.try_send(DeliveryTrace {
                            sequence: stats.completed+1, published_at,
                        }).context("Local delivery trace window exceeded")?;
                    }
                    ensure!(done.reply.send(Ok(record)).is_ok(), "Diagnostic response dropped");
                    stats.completed += 1;
                    stats.encode += done.elapsed;
                    stats.queue.push(done.queued_for);
                    stats.encode_times.push(done.elapsed);
                    stats.publication.push(published_at.duration_since(done.finished_at));
                    lane.advance()?;
                }
                request = requests.recv(), if available && gate.empty() => {
                    let Some(request) = request else { break; };
                    gate.offer(request)?;
                }
                _ = super::host_pipeline::wait_due(staged_due), if staged_due.is_some() => {
                    let source = session.as_mut().context("Missing staged capture session")?;
                    let attempt = pending.as_mut().context("Missing staged demand")?;
                    let polled = source.poll_staged(|| {
                        check_source(child, target, started)?;
                        ensure!(!attempt.request.reply.is_closed() && !server.is_finished(), "Staged diagnostic demand canceled");
                        Ok(())
                    })?;
                    match polled {
                        Poll::Pending => attempt.next_poll = Instant::now() + STAGED_POLL,
                        Poll::Ready((frame, breakdown)) => {
                            check_source(child, target, started)?;
                            source.check_health()?;
                            stats.submit_capture(pending.take().context("Missing completed staging demand")?, frame, breakdown, &mut lane)?;
                        }
                    }
                }
                _ = super::host_pipeline::wait_due(due), if available && due.is_some() => {
                    let mut request = gate.take()?;
                    ensure!(!request.reply.is_closed() && request.desktop_config == Some(config), "Invalid local diagnostic demand");
                    if stats.submitted == samples {
                        ensure!(stats.completed == samples && !lane.busy(), "Unfinished diagnostic encoder");
                        gate.begin(&mut request)?;
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
                    let due = due.context("Missing diagnostic capture deadline")?;
                    let began = gate.begin(&mut request)?;
                    stats.planned.push(due.saturating_duration_since(request.issued_at));
                    stats.late.push(began.saturating_duration_since(due.max(request.issued_at)));
                    if let Some(previous) = last_capture { stats.cadence.push(began.duration_since(previous)); }
                    last_capture = Some(began);
                    let attempt = CaptureAttempt { request, began, overlapping, next_poll: Instant::now() };
                    if policy.staged_readback {
                        source.start_staged()?;
                        pending = Some(attempt);
                    } else {
                        let (frame, breakdown) = source.capture_profiled(|| {
                            check_source(child,target,started)?;
                            ensure!(!attempt.request.reply.is_closed() && !server.is_finished(), "Diagnostic demand canceled");
                            Ok(())
                        }, policy.bounded_lock_wait)?;
                        check_source(child,target,started)?;
                        source.check_health()?;
                        stats.submit_capture(attempt, frame, breakdown, &mut lane)?;
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(8)) => {}
            }
        }
        ensure!(stats.submitted == samples && stats.completed == samples && !stats.elapsed.is_zero(), "Incomplete local diagnostic run");
        Ok::<(),anyhow::Error>(())
    }.await;
    // Drop all pending demand handles and join CPU/native work on every exit.
    gate.cancel();
    pending.take();
    let native = session.as_mut().map_or(Ok(()), CaptureSession::stop);
    let joined = lane.stop().await;
    result?;
    native?;
    joined?;
    Ok(stats)
}

async fn run_one(overlap: bool, verify: bool, policy: CapturePolicy) -> Result<()> {
    let CapturePolicy {
        bounded_lock_wait,
        staged_readback,
        samples,
    } = policy;
    let config = VideoConfig::new(1080, 60, 75, 12)
        .unwrap()
        .with_mode(CodecMode::Compact);
    let mut child = Fixture(
        Command::new(std::env::current_exe()?)
            .args([
                "desktop-test-pattern",
                "--height",
                "1080",
                "--visual-pattern",
            ])
            .spawn()
            .context("Start owned labeled visual diagnostic")?,
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
    let (reference_tx, mut reference_rx) =
        mpsc::channel::<Reference>(intralink_core::desktop::MAX_IN_FLIGHT_RECORDS);
    let (trace_tx, mut trace_rx) =
        mpsc::channel::<DeliveryTrace>(intralink_core::desktop::MAX_IN_FLIGHT_RECORDS);
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
        let mut fidelity = Fidelity::default();
        let mut timing = ViewerTiming::default();
        let received = view(
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
                    let trace = trace_rx
                        .try_recv()
                        .context("Missing local publication trace")?;
                    ensure!(
                        trace.sequence == frame.sequence,
                        "Publication trace sequence mismatch"
                    );
                    let arrived = Instant::now();
                    timing
                        .delivery
                        .push(arrived.duration_since(trace.published_at));
                    decoder.decode_into(config, &frame.payload, &mut pixels)?;
                    timing.decode.push(arrived.elapsed());
                    visual_fixture::verify_decoded(config, &pixels)?;
                    if verify {
                        let reference = reference_rx
                            .try_recv()
                            .context("Missing ordered capture reference")?;
                        ensure!(
                            reference.sequence == frame.sequence
                                && reference.tag == frame.payload[0],
                            "Visual reference sequence/tag mismatch"
                        );
                        fidelity.check(config, &reference, &pixels)?;
                    }
                    Ok(())
                })())
            },
        )
        .await?;
        ensure!(reference_rx.try_recv().is_err(), "Unused visual reference");
        ensure!(trace_rx.try_recv().is_err(), "Unused publication trace");
        Ok::<_, anyhow::Error>((received, fidelity, timing))
    });
    let observations = produce(
        config,
        &mut child,
        &target,
        requests,
        &server,
        TraceChannels {
            references: verify.then_some(reference_tx),
            traces: trace_tx,
        },
        policy,
    )
    .await;
    if observations.is_err() {
        server.abort();
        viewer.abort();
    }
    let (served, viewed) = tokio::join!(server, viewer);
    host.close().await;
    controller.close().await;
    let local = observations?;
    let served = served??;
    let (viewed, fidelity, viewer_timing) = viewed??;
    ensure!(
        served == viewed && served.frames + served.repeats + served.no_updates == samples,
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
    let breakdown = &local.capture_breakdown;
    ensure!(
        breakdown.len() == samples && local.native_times.len() == samples,
        "Missing capture timing samples"
    );
    let reads: Vec<_> = breakdown
        .iter()
        .filter_map(|sample| sample.readback)
        .collect();
    ensure!(
        reads.len() == local.native,
        "Native readback timing count differs"
    );
    for (sample, elapsed) in breakdown.iter().zip(&local.native_times) {
        ensure!(
            sample.generation_wait + sample.contention_wait + sample.staged_wait + sample.consume
                <= *elapsed,
            "Overlapping native timing intervals"
        );
        if let Some(read) = sample.readback {
            ensure!(
                sample.scale_submit + read.copy_submit + read.map_wait + read.cpu_copy
                    <= sample.consume,
                "Overlapping readback timing intervals"
            );
        }
    }
    if verify {
        ensure!(
            fidelity.frames == viewed.frames,
            "Not every changed frame was compared"
        );
        println!(
            "visual_fidelity pipeline={overlap}, bounded_lock_wait={bounded_lock_wait}, staged_readback={staged_readback}, frames={}, pixels={}, lossless_frames={}, max_channel_mae={:.4}, max_block_mae={:.4}, max_large_error_fraction={:.6}",
            fidelity.frames,
            fidelity.pixels,
            fidelity.lossless_frames,
            fidelity.max_channel_mae,
            fidelity.max_block_mae,
            fidelity.max_large_fraction
        );
        return Ok(());
    }
    println!(
        "pipeline={overlap}, bounded_lock_wait={bounded_lock_wait}, staged_readback={staged_readback}, records={samples}, changed={}, repeats={}, no_updates={}, native_samples={}, overlapping_captures={}, elapsed_s={:.3}, records_fps={:.2}, changed_fps={:.2}, capture_ms={:.3}, encode_ms={:.3}, media_bytes={}",
        served.frames,
        served.repeats,
        served.no_updates,
        local.native,
        local.overlapping,
        local.elapsed.as_secs_f64(),
        samples as f64 / local.elapsed.as_secs_f64(),
        served.frames as f64 / local.elapsed.as_secs_f64(),
        local.capture.as_secs_f64() * 1000.0,
        local.encode.as_secs_f64() * 1000.0,
        served.media_bytes
    );
    ensure!(
        viewer_timing.delivery.len() == served.frames
            && viewer_timing.decode.len() == served.frames,
        "Missing changed-frame timing"
    );
    for (label, values) in [
        ("planned_wait", &local.planned),
        ("start_lateness", &local.late),
        ("actual_capture_cadence", &local.cadence),
        ("native_capture", &local.native_times),
        ("encoder_queue", &local.queue),
        ("encode", &local.encode_times),
        ("publication_delay", &local.publication),
        ("local_delivery", &viewer_timing.delivery),
        ("viewer_decode", &viewer_timing.decode),
    ] {
        timing(label, values);
    }
    println!(
        "capture_contention_polls={}, affected_attempts={}",
        breakdown
            .iter()
            .map(|sample| sample.contention_polls)
            .sum::<usize>(),
        breakdown
            .iter()
            .filter(|sample| sample.contention_polls > 0)
            .count()
    );
    println!(
        "staged_map_polls={}, staged_wait_ms={:.3}",
        breakdown.iter().map(|s| s.staged_polls).sum::<usize>(),
        breakdown
            .iter()
            .map(|s| s.staged_wait)
            .sum::<Duration>()
            .as_secs_f64()
            * 1000.0
    );
    for (label, values) in [
        (
            "staged_interpoll_wait",
            breakdown.iter().map(|s| s.staged_wait).collect::<Vec<_>>(),
        ),
        (
            "generation_notification_wait",
            breakdown
                .iter()
                .map(|s| s.generation_wait)
                .collect::<Vec<_>>(),
        ),
        (
            "callback_contention_wait",
            breakdown.iter().map(|s| s.contention_wait).collect(),
        ),
        (
            "native_consume",
            breakdown.iter().map(|s| s.consume).collect(),
        ),
        (
            "scale_submission",
            breakdown
                .iter()
                .filter(|s| s.readback.is_some())
                .map(|s| s.scale_submit)
                .collect(),
        ),
        (
            "copy_submission",
            reads.iter().map(|s| s.copy_submit).collect(),
        ),
        ("map_wait", reads.iter().map(|s| s.map_wait).collect()),
        (
            "cpu_row_copy_unmap",
            reads.iter().map(|s| s.cpu_copy).collect(),
        ),
    ] {
        timing(label, &values);
    }
    Ok(())
}

pub(super) async fn run(verify: bool, compare_lock: bool, compare_staged: bool) -> Result<()> {
    super::require_graphical_environment()?;
    println!(
        "Local labeled visual test window only. No real monitor capture. Focus window and press ESC to cancel."
    );
    let baseline = CapturePolicy {
        bounded_lock_wait: false,
        staged_readback: false,
        samples: SAMPLES,
    };
    if compare_staged {
        for staged_readback in [false, true, true, false] {
            run_one(
                true,
                false,
                CapturePolicy {
                    staged_readback,
                    samples: STAGED_SAMPLES,
                    ..baseline
                },
            )
            .await?;
        }
        run_one(
            true,
            true,
            CapturePolicy {
                staged_readback: true,
                ..baseline
            },
        )
        .await?;
    } else if compare_lock {
        // Keep transport, scene and quality fixed while comparing only the
        // callback lock policy. Validate every changed pixel separately.
        for bounded_lock_wait in [false, true, true, false] {
            run_one(
                true,
                false,
                CapturePolicy {
                    bounded_lock_wait,
                    ..baseline
                },
            )
            .await?;
        }
        run_one(
            true,
            true,
            CapturePolicy {
                bounded_lock_wait: true,
                ..baseline
            },
        )
        .await?;
    } else {
        run_one(false, verify, baseline).await?;
        run_one(true, verify, baseline).await?;
        if !verify {
            // Alternate order to expose warm-up and load variation.
            run_one(true, false, baseline).await?;
            run_one(false, false, baseline).await?;
        }
    }
    println!(
        "{}=passed; generated owned window and authenticated QUIC loopback only.",
        if compare_staged {
            "staged_readback_check"
        } else if compare_lock {
            "capture_lock_check"
        } else if verify {
            "visual_check"
        } else {
            "pipeline_check"
        }
    );
    Ok(())
}
