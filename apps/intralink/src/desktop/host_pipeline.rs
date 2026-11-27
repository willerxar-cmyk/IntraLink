//! Local ordered CPU lane for a fixed, explicitly approved desktop session.
//! The GUI owns capture, cancellation, identity checks and publication.
use anyhow::{Context, Result, ensure};
use image::RgbaImage;
use intralink_core::desktop::VideoConfig;
use intralink_transport::desktop::{Capture, CaptureRequest, Stats};
use minifb::{Key, Window};
use std::time::{Duration, Instant};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use super::{
    codec::Encoder,
    host_capture::{CaptureBackend, HostCapture, HostDisplay},
};

/// Single foreground start gate. Transport grants carry an absolute earliest
/// start; the local fixed-rate check uses the same actual start clock.
pub(super) struct DemandGate {
    pending: Option<CaptureRequest>,
    next: Instant,
    interval: Duration,
}

impl DemandGate {
    pub(super) fn new(config: VideoConfig) -> Self {
        Self {
            pending: None,
            next: Instant::now(),
            interval: Duration::from_secs_f64(1.0 / f64::from(config.fps())),
        }
    }

    pub(super) fn empty(&self) -> bool {
        self.pending.is_none()
    }

    pub(super) fn offer(&mut self, request: CaptureRequest) -> Result<()> {
        ensure!(self.empty(), "Local capture grant slot occupied");
        ensure!(!request.reply.is_closed(), "Local capture demand canceled");
        self.pending = Some(request);
        Ok(())
    }

    pub(super) fn due(&self) -> Option<Instant> {
        self.pending.as_ref().map(|request| {
            request
                .not_before()
                .map_or(self.next, |due| due.max(self.next))
        })
    }

    pub(super) fn take(&mut self) -> Result<CaptureRequest> {
        ensure!(
            self.due().is_some_and(|due| Instant::now() >= due),
            "Capture grant not due"
        );
        self.pending.take().context("Missing capture grant")
    }

    pub(super) fn begin(&mut self, request: &mut CaptureRequest) -> Result<Instant> {
        ensure!(
            Instant::now() >= self.next,
            "Capture started before local FPS interval"
        );
        let started = request.begin_capture()?;
        self.next = started + self.interval;
        Ok(started)
    }

    pub(super) fn cancel(&mut self) {
        self.pending.take();
    }
}

/// Await one absolute deadline, without a second relative timer or
/// waiting for a timer tick when the grant is already due.
pub(super) async fn wait_due(due: Option<Instant>) {
    if let Some(due) = due
        && due > Instant::now()
    {
        tokio::time::sleep_until(tokio::time::Instant::from_std(due)).await;
    }
}

struct Job {
    frame: Option<RgbaImage>,
    request: CaptureRequest,
    submitted: Instant,
}

pub(super) struct Completed {
    pub(super) frame: Option<RgbaImage>,
    pub(super) reply: oneshot::Sender<Result<Capture>>,
    pub(super) record: Result<Capture>,
    pub(super) elapsed: Duration,
    pub(super) queued_for: Duration,
    pub(super) finished_at: Instant,
}

pub(super) struct EncodeLane {
    encoder: Option<Encoder>,
    worker: Option<JoinHandle<(Encoder, Completed)>>,
    queued: Option<Job>,
}

impl EncodeLane {
    pub(super) fn new(config: VideoConfig) -> Self {
        Self {
            encoder: Some(Encoder::new(config)),
            worker: None,
            queued: None,
        }
    }

    pub(super) fn busy(&self) -> bool {
        self.worker.is_some()
    }
    pub(super) fn encoding(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    }
    pub(super) fn can_accept(&self) -> bool {
        self.queued.is_none()
    }

    fn launch(&mut self, job: Job) -> Result<()> {
        ensure!(self.worker.is_none(), "Encoder lane already running");
        let mut encoder = self.encoder.take().context("Missing ordered encoder")?;
        self.worker = Some(tokio::task::spawn_blocking(move || {
            let Job {
                frame,
                request,
                submitted,
            } = job;
            let CaptureRequest { buffer, reply, .. } = request;
            let began = Instant::now();
            let record = if reply.is_closed() {
                Err(anyhow::anyhow!("Local capture demand canceled"))
            } else {
                match &frame {
                    Some(image) => encoder.encode(image, buffer),
                    None => Ok(Capture::NoUpdate(buffer)),
                }
            };
            let finished_at = Instant::now();
            let completed = Completed {
                frame,
                reply,
                record,
                elapsed: finished_at.duration_since(began),
                queued_for: began.duration_since(submitted),
                finished_at,
            };
            (encoder, completed)
        }));
        Ok(())
    }

    pub(super) fn submit(
        &mut self,
        frame: Option<RgbaImage>,
        request: CaptureRequest,
    ) -> Result<()> {
        ensure!(!request.reply.is_closed(), "Local capture demand canceled");
        ensure!(self.can_accept(), "Ordered capture lane is full");
        let job = Job {
            frame,
            request,
            submitted: Instant::now(),
        };
        if self.busy() {
            self.queued = Some(job);
            Ok(())
        } else {
            self.launch(job)
        }
    }

    /// Await in a cancellation-safe way: the handle remains owned by the
    /// lane until completion, even when a GUI timer wins the select.
    pub(super) async fn complete(&mut self) -> Result<Completed> {
        let result = self.worker.as_mut().context("No active encoder")?.await;
        self.worker = None;
        let (encoder, completed) = result.context("Local encoder worker failed")?;
        self.encoder = Some(encoder);
        Ok(completed)
    }

    /// Called only after the GUI has validated and handed off the preceding
    /// result. The next delta must never advance an unpublished base.
    pub(super) fn advance(&mut self) -> Result<()> {
        if let Some(job) = self.queued.take() {
            self.launch(job)?;
        }
        Ok(())
    }

    pub(super) async fn stop(&mut self) -> Result<()> {
        // Discard the queued raw frame and reply. Never publish on shutdown.
        self.queued.take();
        if let Some(worker) = self.worker.take() {
            worker.await.context("Join local encoder worker")?;
        }
        self.encoder.take();
        Ok(())
    }
}

pub(super) async fn run(
    display: &HostDisplay,
    backend: CaptureBackend,
    config: VideoConfig,
    indicator: &mut Window,
    requests: &mut mpsc::Receiver<CaptureRequest>,
    task: &JoinHandle<Result<Option<Stats>>>,
    stop: oneshot::Sender<()>,
) -> Result<()> {
    let mut source = None::<HostCapture>;
    let mut lane = None::<EncodeLane>;
    let mut last_paint = Instant::now();
    let mut last_health = Instant::now();
    let mut gate = DemandGate::new(config);
    let mut overlaps = 0u64;
    let mut captures = 0u64;
    let mut completed = 0u64;
    let mut encode_time = Duration::ZERO;
    let mut capture_time = Duration::ZERO;
    let mut encode_queue = Duration::ZERO;
    let mut publish_delay = Duration::ZERO;
    let sharing = vec![0x00c52d3b; 320 * 64];
    let local = async {
        loop {
            indicator.update();
            if task.is_finished() || !indicator.is_open() || indicator.is_key_down(Key::Escape) {
                break;
            }
            if source.is_some() && last_paint.elapsed() >= Duration::from_millis(100) {
                indicator.update_with_buffer(&sharing, 320, 64)
                    .map_err(|error| anyhow::anyhow!("Sharing indicator: {error}"))?;
                last_paint = Instant::now();
            }
            if last_health.elapsed() >= Duration::from_millis(250) {
                if let Some(source) = &source {
                    display.verify()?;
                    source.check_health()?;
                }
                last_health = Instant::now();
            }
            let busy = lane.as_ref().is_some_and(EncodeLane::busy);
            let available = lane.as_ref().is_none_or(EncodeLane::can_accept);
            let due = gate.due();

            tokio::select! {
                biased;
                done = async { lane.as_mut().context("Missing encoder lane")?.complete().await }, if busy => {
                    let result = done?;
                    let active = source.as_mut().context("Missing capture source")?;
                    if let Some(frame) = result.frame { active.recycle(frame); }
                    // A worker never owns a native handle or the right to
                    // publish pixels. Revalidate source and local consent here.
                    indicator.update();
                    if !indicator.is_open() || indicator.is_key_down(Key::Escape)
                        || task.is_finished() || result.reply.is_closed() {
                        break;
                    }
                    display.verify()?;
                    active.check_health()?;
                    let record = result.record?;
                    indicator.update();
                    if !indicator.is_open() || indicator.is_key_down(Key::Escape)
                        || task.is_finished() || result.reply.is_closed() {
                        break;
                    }
                    publish_delay += result.finished_at.elapsed();
                    if result.reply.send(Ok(record)).is_err() { break; }
                    completed += 1;
                    encode_time += result.elapsed;
                    encode_queue += result.queued_for;
                    lane.as_mut().context("Missing encoder lane")?.advance()?;
                }
                request = requests.recv(), if available && gate.empty() => {
                    let Some(request) = request else { break; };
                    gate.offer(request)?;
                }
                _ = wait_due(due), if available && due.is_some() => {
                    let mut request = gate.take()?;
                    indicator.update();
                    if !indicator.is_open() || indicator.is_key_down(Key::Escape)
                        || task.is_finished() || request.reply.is_closed() {
                        break;
                    }
                    ensure!(request.desktop_config == Some(config), "Pipelined media profile changed after approval");
                    if source.is_none() {
                        indicator.set_title("IntraLink: SCREEN SHARING - fixed pipeline - ESC to stop");
                        indicator.update_with_buffer(&sharing, 320, 64)
                            .map_err(|error| anyhow::anyhow!("Sharing indicator: {error}"))?;
                        if !indicator.is_open() || indicator.is_key_down(Key::Escape) { break; }
                        source = Some(HostCapture::start(display, backend, config)?);
                        lane = Some(EncodeLane::new(config));
                        last_paint = Instant::now();
                        println!("SCREEN SHARING ACTIVE: ordered two-demand pipeline. ESC to stop.");
                    }
                    let active = source.as_mut().context("Missing capture source")?;
                    let overlapping = lane.as_ref().is_some_and(EncodeLane::encoding);
                    let began = gate.begin(&mut request)?;
                    // Capture occurs only after a reserved authenticated
                    // demand, and never more rapidly than the approved FPS.
                    let frame = active.capture(display, || {
                        indicator.update();
                        ensure!(indicator.is_open() && !indicator.is_key_down(Key::Escape)
                            && !request.reply.is_closed() && !task.is_finished(), "Local sharing stopped");
                        Ok(())
                    });
                    capture_time += began.elapsed();
                    if !indicator.is_open() || indicator.is_key_down(Key::Escape)
                        || request.reply.is_closed() || task.is_finished() { break; }
                    let frame = frame?;
                    captures += u64::from(frame.is_some());
                    overlaps += u64::from(overlapping && frame.is_some());
                    lane.as_mut().context("Missing encoder lane")?.submit(frame, request)?;
                }
                _ = tokio::time::sleep(Duration::from_millis(8)) => {}
            }
        }
        Ok::<(), anyhow::Error>(())
    }.await;
    // Cancel network before any potentially blocking native/CPU cleanup.
    if !task.is_finished() {
        let _ = stop.send(());
    }
    gate.cancel();
    let native = source.as_mut().map_or(Ok(()), HostCapture::stop);
    let joined = match &mut lane {
        Some(lane) => lane.stop().await,
        None => Ok(()),
    };
    println!(
        "Local ordered pipeline: captures={captures}, completed={completed}, overlapping_captures={overlaps}, capture_ms={:.3}, encode_ms={:.3}, encode_queue_ms={:.3}, publish_delay_ms={:.3}",
        capture_time.as_secs_f64() * 1000.0,
        encode_time.as_secs_f64() * 1000.0,
        encode_queue.as_secs_f64() * 1000.0,
        publish_delay.as_secs_f64() * 1000.0
    );
    local?;
    native?;
    joined
}

#[cfg(test)]
mod tests {
    use super::super::codec::Decoder;
    use super::*;
    use intralink_core::{
        desktop::CodecMode, media::LOSSLESS_KEYFRAME_TAG, quality::ReceiptPressure,
    };

    fn request(config: VideoConfig) -> (CaptureRequest, oneshot::Receiver<Result<Capture>>) {
        let (reply, response) = oneshot::channel();
        (
            CaptureRequest {
                buffer: Vec::new(),
                reply,
                receipt_pressure: ReceiptPressure::Neutral,
                desktop_config: Some(config),
                issued_at: Instant::now(),
                permit: None,
            },
            response,
        )
    }

    #[test]
    fn demand_gate_keeps_one_slot_and_never_starts_early() -> Result<()> {
        let config = VideoConfig::new(540, 60, 75, 30).unwrap();
        let mut gate = DemandGate::new(config);
        let (first, first_response) = request(config);
        let (second, second_response) = request(config);
        gate.offer(first)?;
        assert!(gate.offer(second).is_err());
        assert!(second_response.blocking_recv().is_err());
        gate.next = Instant::now() + Duration::from_secs(5);
        assert!(gate.take().is_err());
        gate.next = Instant::now();
        let mut first = gate.take()?;
        let actual = gate.begin(&mut first)?;
        assert_eq!(gate.next, actual + Duration::from_secs_f64(1.0 / 60.0));
        gate.next = Instant::now() + Duration::from_secs(5);
        assert!(gate.begin(&mut first).is_err());
        drop(first);
        assert!(first_response.blocking_recv().is_err());
        let (third, third_response) = request(config);
        gate.offer(third)?;
        gate.cancel();
        assert!(gate.empty());
        assert!(third_response.blocking_recv().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn lane_holds_only_two_and_keeps_exact_delta_base() -> Result<()> {
        let config = VideoConfig::new(540, 60, 75, 30)
            .unwrap()
            .with_mode(CodecMode::Compact);
        let mut lane = EncodeLane::new(config);
        let (first, first_response) = request(config);
        let (second, second_response) = request(config);
        let image = RgbaImage::from_pixel(960, 540, image::Rgba([21, 31, 41, 255]));
        let mut changed = image.clone();
        changed.put_pixel(0, 0, image::Rgba([71, 81, 91, 255]));
        lane.submit(Some(image), first)?;
        lane.submit(Some(changed), second)?;
        let (third, _third_response) = request(config);
        assert!(lane.submit(None, third).is_err());
        let first = lane.complete().await?;
        assert!(first.reply.send(first.record).is_ok());
        assert!(!lane.busy());
        lane.advance()?;
        let second = lane.complete().await?;
        assert!(second.reply.send(second.record).is_ok());
        let Capture::Frame(key) = first_response.await?? else {
            anyhow::bail!("Missing keyframe");
        };
        assert_eq!(key[0], LOSSLESS_KEYFRAME_TAG);
        let Capture::Frame(delta) = second_response.await?? else {
            anyhow::bail!("Missing delta");
        };
        let mut decoder = Decoder::default();
        let mut pixels = Vec::new();
        decoder.decode_into(config, &key, &mut pixels)?;
        decoder.decode_into(config, &delta, &mut pixels)?;
        assert_eq!(pixels[0], 0x47515b);
        assert_eq!(pixels[1], 0x151f29);
        lane.stop().await
    }

    #[tokio::test]
    async fn stop_joins_active_and_discards_queued_without_publication() -> Result<()> {
        let config = VideoConfig::new(540, 60, 75, 30).unwrap();
        let mut lane = EncodeLane::new(config);
        let (first, a) = request(config);
        let (second, b) = request(config);
        lane.submit(
            Some(RgbaImage::from_pixel(960, 540, image::Rgba([1, 2, 3, 255]))),
            first,
        )?;
        lane.submit(None, second)?;
        lane.stop().await?;
        assert!(a.await.is_err());
        assert!(b.await.is_err());
        assert!(!lane.busy());
        Ok(())
    }
}
