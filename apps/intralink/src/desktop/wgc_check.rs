//! Persistent WGC session with a one-surface GPU mailbox.
//! Host callers must enforce local display approval before construction.
use std::{
    sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    task::Poll,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, ensure};
use image::RgbaImage;
use intralink_d3d11::{GpuScaler, LatestSurface, Readback, ReadbackTiming};
use windows_capture::{
    capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
    window::Window,
};

use super::codec::validate_source_size;

pub(super) type Captured = (RgbaImage, (u32, u32));

#[derive(Default)]
pub(super) struct CaptureTiming {
    pub generation_wait: Duration,
    pub contention_wait: Duration,
    pub contention_polls: usize,
    pub consume: Duration,
    pub scale_submit: Duration,
    pub readback: Option<ReadbackTiming>,
    pub staged_wait: Duration,
    pub staged_polls: usize,
}

struct Flags {
    wake: SyncSender<()>,
    expected_source: Option<(u32, u32)>,
}

#[derive(Default, Clone, Copy)]
pub(super) struct CaptureStats {
    pub staging_allocations: u64,
    pub rgba_allocations: u64,
    pub gpu_processors: u64,
    pub readback_bytes: u64,
    pub native_updates: u64,
    pub readback_frames: u64,
}

struct Handler {
    flags: Flags,
    latest: Option<LatestSurface>,
    readback: Readback,
    scaler: Option<GpuScaler>,
    stats: CaptureStats,
    failed: Option<String>,
    closed: bool,
}

impl Handler {
    fn update(&mut self, frame: &Frame<'_>) -> Result<()> {
        validate_source_size(frame.width(), frame.height())?;
        ensure!(
            self.flags
                .expected_source
                .is_none_or(|size| size == (frame.width(), frame.height())),
            "Capture does not match approved display geometry"
        );
        if let Some(surface) = &mut self.latest {
            surface.update(frame)?;
        } else {
            self.latest = Some(LatestSurface::new(frame)?);
        }
        self.stats.native_updates = self
            .stats
            .native_updates
            .checked_add(1)
            .context("Native frame generation exhausted")?;
        Ok(())
    }

    // Both paths prepare the same retained scaler and enforce the same
    // generation and device checks while the callback mutex is held.
    fn prepare(
        &mut self,
        after: u64,
        gpu_target: Option<(u32, u32)>,
    ) -> Result<Option<FrameGeometry>> {
        ensure!(
            !self.readback.is_pending(),
            "Previous staging copy still pending"
        );
        ensure!(!self.closed, "WGC source closed");
        if let Some(error) = &self.failed {
            anyhow::bail!("WGC source failed: {error}");
        }
        let Some(surface) = &self.latest else {
            return Ok(None);
        };
        surface.check_device()?;
        if self.stats.native_updates <= after {
            return Ok(None);
        }
        let native_size = surface.size();
        if let Some(target) = gpu_target {
            let fit = super::resize::fitted(native_size, target);
            if self
                .scaler
                .as_ref()
                .is_none_or(|scaler| scaler.output_size() != fit)
            {
                // Build replacement on the same verified capture device.
                // Release the old output staging only after construction succeeds.
                let next = GpuScaler::new(surface, fit)?;
                if self.scaler.is_some() {
                    self.readback.reset_output_staging()?;
                }
                self.scaler = Some(next);
                self.stats.gpu_processors += 1;
            }
        } else {
            ensure!(self.scaler.is_none(), "GPU capture mode changed");
        }
        let output = self
            .scaler
            .as_ref()
            .map_or(native_size, GpuScaler::output_size);
        Ok(Some(FrameGeometry {
            generation: self.stats.native_updates,
            native: native_size,
            output,
        }))
    }

    fn consume(
        &mut self,
        after: u64,
        rgba: &mut Vec<u8>,
        gpu_target: Option<(u32, u32)>,
        timing: &mut CaptureTiming,
    ) -> Result<Option<(u64, Captured)>> {
        let Some(geometry) = self.prepare(after, gpu_target)? else {
            return Ok(None);
        };
        let surface = self.latest.as_ref().context("Missing prepared source")?;
        if rgba.capacity() < geometry.output.0 as usize * geometry.output.1 as usize * 4 {
            self.stats.rgba_allocations += 1;
        }
        if let Some(scaler) = &mut self.scaler {
            timing.scale_submit = scaler.scale_and_read(surface, &mut self.readback, rgba)?;
        } else {
            surface.read(&mut self.readback, rgba)?;
        }
        self.finish_image(geometry, rgba, timing).map(Some)
    }

    fn submit_staged(
        &mut self,
        after: u64,
        target: (u32, u32),
        timing: &mut CaptureTiming,
    ) -> Result<Option<FrameGeometry>> {
        let Some(geometry) = self.prepare(after, Some(target))? else {
            return Ok(None);
        };
        let surface = self.latest.as_ref().context("Missing prepared source")?;
        timing.scale_submit = self
            .scaler
            .as_mut()
            .context("Missing staged scaler")?
            .submit_readback(surface, &mut self.readback)?;
        Ok(Some(geometry))
    }

    fn poll_staged(
        &mut self,
        geometry: FrameGeometry,
        rgba: &mut Vec<u8>,
        timing: &mut CaptureTiming,
    ) -> Result<Option<(u64, Captured)>> {
        ensure!(!self.closed, "WGC source closed");
        if let Some(error) = &self.failed {
            anyhow::bail!("WGC source failed: {error}");
        }
        let surface = self.latest.as_ref().context("Missing staged source")?;
        surface.check_device()?;
        ensure!(
            surface.size() == geometry.native && self.stats.native_updates >= geometry.generation,
            "Staged source geometry/generation changed"
        );
        ensure!(
            self.scaler
                .as_ref()
                .is_some_and(|scaler| scaler.output_size() == geometry.output),
            "Staged scaler changed"
        );
        let allocation =
            rgba.capacity() < geometry.output.0 as usize * geometry.output.1 as usize * 4;
        if !self.readback.try_finish(rgba)? {
            return Ok(None);
        }
        self.stats.rgba_allocations += u64::from(allocation);
        self.finish_image(geometry, rgba, timing).map(Some)
    }

    fn finish_image(
        &mut self,
        geometry: FrameGeometry,
        rgba: &mut Vec<u8>,
        timing: &mut CaptureTiming,
    ) -> Result<(u64, Captured)> {
        timing.readback = Some(self.readback.last_timing());
        self.stats.staging_allocations = self.readback.allocations();
        self.stats.readback_bytes += rgba.len() as u64;
        self.stats.readback_frames += 1;
        let (width, height) = geometry.output;
        let image = RgbaImage::from_raw(width, height, std::mem::take(rgba))
            .context("Invalid GPU readback geometry")?;
        Ok((geometry.generation, (image, geometry.native)))
    }
}

#[derive(Clone, Copy)]
struct FrameGeometry {
    generation: u64,
    native: (u32, u32),
    output: (u32, u32),
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Flags;
    type Error = String;

    fn new(context: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            flags: context.flags,
            latest: None,
            readback: Readback::default(),
            scaler: None,
            stats: CaptureStats::default(),
            failed: None,
            closed: false,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame<'_>,
        control: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        // Always retain the latest native update on the GPU. A full wake queue
        // coalesces notifications, never drops the latest surface itself.
        if let Err(error) = self.update(frame) {
            let error = format!("{error:#}");
            self.failed = Some(error.clone());
            let _ = self.flags.wake.try_send(());
            return Err(error);
        }
        if let Err(TrySendError::Disconnected(_)) = self.flags.wake.try_send(()) {
            control.stop();
        }
        Ok(())
    }

    fn on_closed(&mut self) -> std::result::Result<(), Self::Error> {
        self.closed = true;
        let _ = self.flags.wake.try_send(());
        Ok(())
    }
}

// At most one authorized generation can own staging. It is never retargeted
// or replaced while the consumer is waiting for GPU completion.
struct StagedCapture {
    geometry: Option<FrameGeometry>,
    deadline: Instant,
    timing: CaptureTiming,
    last_poll_end: Option<Instant>,
    contended: bool,
}

pub(super) struct CaptureSession {
    control: Option<CaptureControl<Handler, String>>,
    wake: Receiver<()>,
    delivered: u64,
    rgba: Vec<u8>,
    gpu_ceiling: Option<(u32, u32)>,
    gpu_target: Option<(u32, u32)>,
    staged: Option<StagedCapture>,
}

impl CaptureSession {
    pub(super) fn start_fixture(
        pid: u32,
        title: &str,
        gpu_target: Option<(u32, u32)>,
    ) -> Result<Self> {
        let mut matches = Window::enumerate()?.into_iter().filter(|window| {
            window.process_id().ok() == Some(pid) && window.title().ok().as_deref() == Some(title)
        });
        let target = matches
            .next()
            .context("Own diagnostic WGC window was not found")?;
        ensure!(
            matches.next().is_none(),
            "Ambiguous owned diagnostic WGC window"
        );
        ensure!(target.is_valid(), "Diagnostic WGC window is unavailable");
        Self::start(target, gpu_target, None)
    }

    // Match an enumerated native handle exactly, never construct it from a CLI
    // number or guess based on display index. XCap's Windows ID is only u32.
    pub(super) fn start_monitor(
        id: u32,
        name: &str,
        size: (u32, u32),
        gpu_target: Option<(u32, u32)>,
    ) -> Result<Self> {
        let mut candidates = Monitor::enumerate()?
            .into_iter()
            .filter(|monitor| u32::try_from(monitor.as_raw_hmonitor() as usize).ok() == Some(id));
        let target = candidates
            .next()
            .context("Approved native monitor unavailable")?;
        ensure!(
            candidates.next().is_none(),
            "Ambiguous native monitor identity"
        );
        ensure!(
            target.device_name()? == name && (target.width()?, target.height()?) == size,
            "Native monitor differs from approved selection"
        );
        Self::start(target, gpu_target, Some(size))
    }

    fn start<T: TryInto<GraphicsCaptureItemType> + Send + 'static>(
        target: T,
        gpu_target: Option<(u32, u32)>,
        expected_source: Option<(u32, u32)>,
    ) -> Result<Self> {
        let (tx, wake) = sync_channel(1);
        let settings = Settings::new(
            target,
            CursorCaptureSettings::Default,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Rgba8,
            Flags {
                wake: tx,
                expected_source,
            },
        );
        let control = Handler::start_free_threaded(settings)
            .map_err(|error| anyhow!("Start persistent WGC: {error}"))?;
        Ok(Self {
            control: Some(control),
            wake,
            delivered: 0,
            rgba: Vec::new(),
            gpu_ceiling: gpu_target,
            gpu_target,
            staged: None,
        })
    }

    /// Change only the output scaler target, never the capture item or device.
    /// Applied to the next new generation under the callback lock.
    pub(super) fn retarget(&mut self, target: (u32, u32)) -> Result<()> {
        ensure!(
            self.staged.is_none(),
            "Cannot retarget during staged capture"
        );
        self.check_health()?;
        if let Some(ceiling) = self.gpu_ceiling {
            ensure!(
                matches!(target, (1920, 1080) | (1280, 720) | (960, 540))
                    && target.0 <= ceiling.0
                    && target.1 <= ceiling.1,
                "Scaler target exceeds the approved output geometry"
            );
            self.gpu_target = Some(target);
        }
        Ok(())
    }

    /// Wait for a new observed generation, pumping local cancellation at most
    /// every 8 ms while waiting. A missing update is not an encoded repeat.
    /// Native GPU readback itself is synchronous and may still block.
    pub(super) fn capture(
        &mut self,
        check: impl FnMut() -> Result<()>,
    ) -> Result<Option<Captured>> {
        self.capture_profiled(check, false).map(|(frame, _)| frame)
    }

    /// A local diagnostic may compare a bounded mutex wait against the
    /// original wake-queue behavior. Actual Host policy stays unchanged.
    pub(super) fn capture_profiled(
        &mut self,
        mut check: impl FnMut() -> Result<()>,
        bounded_lock_wait: bool,
    ) -> Result<(Option<Captured>, CaptureTiming)> {
        ensure!(self.staged.is_none(), "Staged capture already active");
        let mut timing = CaptureTiming::default();
        let control = self.control.as_ref().context("WGC session ended")?;
        let callback = control.callback();
        let halt = control.halt_handle();
        let first = self.delivered == 0;
        let limit = if first {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(17)
        };
        let started = Instant::now();
        loop {
            check()?;
            ensure!(
                !control.is_finished() && !halt.load(std::sync::atomic::Ordering::Acquire),
                "Persistent WGC session ended"
            );
            let mut guard = callback.try_lock();
            if guard.is_none() {
                timing.contention_polls += 1;
                if bounded_lock_wait {
                    let remaining = limit.saturating_sub(started.elapsed());
                    if !remaining.is_zero() {
                        let wait = Instant::now();
                        // The callback can signal while it still owns the lock.
                        // Wait for the actual lock, not a second frame notification.
                        guard = callback.try_lock_for(remaining.min(Duration::from_millis(1)));
                        timing.contention_wait += wait.elapsed();
                        check()?;
                        ensure!(
                            !control.is_finished()
                                && !halt.load(std::sync::atomic::Ordering::Acquire),
                            "Persistent WGC session ended"
                        );
                    }
                }
            }
            let contended = guard.is_none();
            if let Some(mut handler) = guard {
                let consuming = Instant::now();
                let frame = handler.consume(
                    self.delivered,
                    &mut self.rgba,
                    self.gpu_target,
                    &mut timing,
                )?;
                timing.consume += consuming.elapsed();
                if let Some((generation, frame)) = frame {
                    check()?;
                    ensure!(
                        !halt.load(std::sync::atomic::Ordering::Acquire),
                        "WGC stopped during readback"
                    );
                    self.delivered = generation;
                    return Ok((Some(frame), timing));
                }
            }
            let remaining = limit.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                ensure!(!first, "WGC first-frame deadline exceeded");
                return Ok((None, timing));
            }
            if bounded_lock_wait && contended {
                // Retry only after the bounded blocking mutex wait. Local
                // cancellation and the original deadline are checked again.
                continue;
            }
            let wait = Instant::now();
            match self
                .wake
                .recv_timeout(remaining.min(Duration::from_millis(8)))
            {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("WGC callback disconnected")
                }
            }
            if contended {
                timing.contention_wait += wait.elapsed();
            } else {
                timing.generation_wait += wait.elapsed();
            }
        }
    }

    /// Diagnostic only: called after an authenticated capture demand has
    /// begun. The caller owns the demand until this generation is complete.
    pub(super) fn start_staged(&mut self) -> Result<()> {
        ensure!(self.staged.is_none(), "Staged capture already active");
        ensure!(
            self.gpu_target.is_some(),
            "Staged capture requires explicit GPU scale"
        );
        self.check_health()?;
        let limit = if self.delivered == 0 {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(17)
        };
        self.staged = Some(StagedCapture {
            geometry: None,
            deadline: Instant::now() + limit,
            timing: CaptureTiming::default(),
            last_poll_end: None,
            contended: false,
        });
        Ok(())
    }

    /// No native wait or busy spin: submit or poll once per caller wake.
    /// The same foreground owner checks source identity and cancellation.
    pub(super) fn poll_staged(
        &mut self,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<Poll<(Option<Captured>, CaptureTiming)>> {
        check()?;
        let mut staged = self.staged.take().context("No staged capture")?;
        let control = self.control.as_ref().context("WGC session ended")?;
        let halt = control.halt_handle();
        ensure!(
            !control.is_finished() && !halt.load(std::sync::atomic::Ordering::Acquire),
            "Persistent WGC session ended"
        );
        if let Some(end) = staged.last_poll_end {
            let elapsed = end.elapsed();
            if staged.geometry.is_some() {
                staged.timing.staged_wait += elapsed;
            } else if staged.contended {
                staged.timing.contention_wait += elapsed;
            } else {
                staged.timing.generation_wait += elapsed;
            }
        }
        // Never turn a failed or slow submitted copy into a no-update.
        if Instant::now() >= staged.deadline {
            ensure!(
                staged.geometry.is_none(),
                "Staged GPU readback deadline exceeded"
            );
            ensure!(self.delivered != 0, "WGC first-frame deadline exceeded");
            return Ok(Poll::Ready((None, staged.timing)));
        }
        let callback = control.callback();
        let mut completed = None;
        if let Some(mut handler) = callback.try_lock() {
            check()?;
            ensure!(
                !halt.load(std::sync::atomic::Ordering::Acquire),
                "WGC stopped before staged readback"
            );
            let began = Instant::now();
            staged.contended = false;
            if let Some(geometry) = staged.geometry {
                staged.timing.staged_polls += 1;
                completed = handler.poll_staged(geometry, &mut self.rgba, &mut staged.timing)?;
            } else if let Some(geometry) = handler.submit_staged(
                self.delivered,
                self.gpu_target.context("Missing staged scale target")?,
                &mut staged.timing,
            )? {
                staged.geometry = Some(geometry);
                // Bound a submitted GPU copy independently of generation search.
                staged.deadline = Instant::now() + Duration::from_millis(100);
            }
            staged.timing.consume += began.elapsed();
        } else {
            staged.contended = true;
            staged.timing.contention_polls += 1;
        }
        check()?;
        ensure!(
            !halt.load(std::sync::atomic::Ordering::Acquire),
            "WGC stopped during staged readback"
        );
        if let Some((generation, frame)) = completed {
            ensure!(
                generation > self.delivered,
                "Staged generation already delivered"
            );
            ensure!(
                Instant::now() < staged.deadline,
                "Staged GPU readback deadline exceeded"
            );
            self.delivered = generation;
            return Ok(Poll::Ready((Some(frame), staged.timing)));
        }
        staged.last_poll_end = Some(Instant::now());
        self.staged = Some(staged);
        Ok(Poll::Pending)
    }

    // A successful probe excludes known closure/device failure only. It
    // does not prove that the compositor is still producing fresh frames.
    pub(super) fn check_health(&self) -> Result<()> {
        let control = self.control.as_ref().context("WGC session ended")?;
        ensure!(
            !control.is_finished()
                && !control
                    .halt_handle()
                    .load(std::sync::atomic::Ordering::Acquire),
            "Persistent WGC session ended"
        );
        if let Some(handler) = control.callback().try_lock() {
            ensure!(!handler.closed, "WGC source closed");
            if let Some(error) = &handler.failed {
                anyhow::bail!("WGC source failed: {error}");
            }
            if let Some(surface) = &handler.latest {
                surface.check_device()?;
            }
        }
        Ok(())
    }

    pub(super) fn recycle(&mut self, image: RgbaImage) {
        self.rgba = image.into_raw();
    }

    pub(super) fn stats(&self) -> Result<CaptureStats> {
        let callback = self
            .control
            .as_ref()
            .context("WGC session ended")?
            .callback();
        let stats = callback.lock().stats;
        Ok(stats)
    }

    /// Observe a callback failure without performing a readback.
    pub(super) fn failure(&self) -> Result<Option<String>> {
        let callback = self
            .control
            .as_ref()
            .context("WGC session ended")?
            .callback();
        let failure = callback
            .try_lock()
            .and_then(|handler| handler.failed.clone());
        Ok(failure)
    }

    /// Observe the actual WGC Closed event without reading a frame.
    /// A false result is not evidence that capture is healthy.
    pub(super) fn source_closed(&self) -> Result<bool> {
        let callback = self
            .control
            .as_ref()
            .context("WGC session ended")?
            .callback();
        let closed = callback.try_lock().is_some_and(|handler| handler.closed);
        Ok(closed)
    }

    pub(super) fn stop(&mut self) -> Result<()> {
        self.staged.take();
        if let Some(control) = self.control.take() {
            control
                .stop()
                .map_err(|error| anyhow!("Stop persistent WGC: {error}"))?;
        }
        Ok(())
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("{error:#}");
        }
    }
}
