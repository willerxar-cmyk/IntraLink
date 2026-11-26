//! Persistent WGC session with a one-surface GPU mailbox.
//! Host callers must enforce local display approval before construction.
use std::{
    sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, ensure};
use image::RgbaImage;
use intralink_d3d11::{GpuScaler, LatestSurface, Readback};
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

    // Called under the same callback lock as update. Never publish an
    // uninitialized frame, old generation, or frame from a failed source.
    fn consume(
        &mut self,
        after: u64,
        rgba: &mut Vec<u8>,
        gpu_target: Option<(u32, u32)>,
    ) -> Result<Option<(u64, Captured)>> {
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
                    self.readback.reset_output_staging();
                }
                self.scaler = Some(next);
                self.stats.gpu_processors += 1;
            }
        } else {
            ensure!(self.scaler.is_none(), "GPU capture mode changed");
        }
        let (width, height) = self
            .scaler
            .as_ref()
            .map_or(native_size, GpuScaler::output_size);
        if rgba.capacity() < width as usize * height as usize * 4 {
            self.stats.rgba_allocations += 1;
        }
        if let Some(scaler) = &mut self.scaler {
            scaler.scale_and_read(surface, &mut self.readback, rgba)?;
        } else {
            surface.read(&mut self.readback, rgba)?;
        }
        self.stats.staging_allocations = self.readback.allocations();
        self.stats.readback_bytes += rgba.len() as u64;
        self.stats.readback_frames += 1;
        let image = RgbaImage::from_raw(width, height, std::mem::take(rgba))
            .context("Invalid GPU readback geometry")?;
        Ok(Some((self.stats.native_updates, (image, native_size))))
    }
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

pub(super) struct CaptureSession {
    control: Option<CaptureControl<Handler, String>>,
    wake: Receiver<()>,
    delivered: u64,
    rgba: Vec<u8>,
    gpu_ceiling: Option<(u32, u32)>,
    gpu_target: Option<(u32, u32)>,
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
        })
    }

    /// Change only the output scaler target, never the capture item or device.
    /// Applied to the next new generation under the callback lock.
    pub(super) fn retarget(&mut self, target: (u32, u32)) -> Result<()> {
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
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<Option<Captured>> {
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
            if let Some(mut handler) = callback.try_lock()
                && let Some((generation, frame)) =
                    handler.consume(self.delivered, &mut self.rgba, self.gpu_target)?
            {
                check()?;
                ensure!(
                    !halt.load(std::sync::atomic::Ordering::Acquire),
                    "WGC stopped during readback"
                );
                self.delivered = generation;
                return Ok(Some(frame));
            }
            let remaining = limit.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                ensure!(!first, "WGC first-frame deadline exceeded");
                return Ok(None);
            }
            match self
                .wake
                .recv_timeout(remaining.min(Duration::from_millis(8)))
            {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("WGC callback disconnected")
                }
            }
        }
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
