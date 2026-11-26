//! Local capture backend selection. Never select a different display or
//! backend implicitly; caller owns the foreground consent and stop indicator.
use anyhow::{Context, Result, anyhow, ensure};
use image::RgbaImage;
use intralink_core::desktop::VideoConfig;
use xcap::Monitor;

#[cfg(all(target_os = "windows", feature = "wgc-preview"))]
use super::wgc_check::CaptureSession;
#[cfg(all(target_os = "windows", feature = "wgc-preview"))]
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum CaptureBackend {
    #[default]
    Snapshot,
    Wgc,
    WgcGpu,
}

impl CaptureBackend {
    pub(super) fn validate(self) -> Result<()> {
        ensure!(
            matches!(self, Self::Snapshot)
                || cfg!(all(target_os = "windows", feature = "wgc-preview")),
            "Selected WGC backend is unavailable in this build"
        );
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
struct Identity {
    id: u32,
    name: String,
    size: (u32, u32),
    origin: (i32, i32),
    rotation: f32,
}

impl Identity {
    fn read(monitor: &Monitor) -> Result<Self> {
        let id = monitor.id().map_err(|e| anyhow!("Display ID: {e}"))?;
        let name = monitor.name().map_err(|e| anyhow!("Display name: {e}"))?;
        let width = monitor.width().map_err(|e| anyhow!("Display width: {e}"))?;
        let height = monitor
            .height()
            .map_err(|e| anyhow!("Display height: {e}"))?;
        super::codec::validate_source_size(width, height)?;
        let origin = (
            monitor.x().map_err(|e| anyhow!("Display x: {e}"))?,
            monitor.y().map_err(|e| anyhow!("Display y: {e}"))?,
        );
        let rotation = monitor
            .rotation()
            .map_err(|e| anyhow!("Display rotation: {e}"))?;
        ensure!(
            !name.is_empty() && rotation.is_finite(),
            "Invalid display identity"
        );
        Ok(Self {
            id,
            name,
            size: (width, height),
            origin,
            rotation,
        })
    }
}

pub(super) struct HostDisplay {
    monitor: Monitor,
    identity: Identity,
}

impl HostDisplay {
    pub(super) fn select(id: u32) -> Result<Self> {
        let mut selected = None;
        for monitor in super::monitors()? {
            if monitor.id().map_err(|e| anyhow!("Display ID: {e}"))? == id {
                ensure!(selected.is_none(), "Ambiguous selected display");
                selected = Some(monitor);
            }
        }
        let monitor = selected.context("Selected display is not available")?;
        let identity = Identity::read(&monitor)?;
        Ok(Self { monitor, identity })
    }

    pub(super) fn name(&self) -> &str {
        &self.identity.name
    }
    pub(super) fn size(&self) -> (u32, u32) {
        self.identity.size
    }

    pub(super) fn verify(&self) -> Result<()> {
        let current = Self::select(self.identity.id)?;
        ensure!(
            current.identity == self.identity,
            "Approved display identity or geometry changed; authorize again"
        );
        Ok(())
    }
}

pub(super) enum HostCapture {
    Snapshot,
    #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
    Wgc {
        session: Box<CaptureSession>,
        last_frame: Instant,
    },
}

impl HostCapture {
    pub(super) fn start(
        display: &HostDisplay,
        backend: CaptureBackend,
        config: VideoConfig,
    ) -> Result<Self> {
        backend.validate()?;
        display.verify()?;
        #[cfg(not(all(target_os = "windows", feature = "wgc-preview")))]
        let _ = config;
        match backend {
            CaptureBackend::Snapshot => Ok(Self::Snapshot),
            #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
            CaptureBackend::Wgc | CaptureBackend::WgcGpu => {
                let target = matches!(backend, CaptureBackend::WgcGpu)
                    .then_some((config.width(), config.height()));
                let session = CaptureSession::start_monitor(
                    display.identity.id,
                    display.name(),
                    display.size(),
                    target,
                )?;
                Ok(Self::Wgc {
                    session: Box::new(session),
                    last_frame: Instant::now(),
                })
            }
            #[cfg(not(all(target_os = "windows", feature = "wgc-preview")))]
            CaptureBackend::Wgc | CaptureBackend::WgcGpu => {
                anyhow::bail!("WGC backend unavailable")
            }
        }
    }

    pub(super) fn check_health(&self) -> Result<()> {
        match self {
            Self::Snapshot => Ok(()),
            #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
            Self::Wgc {
                session,
                last_frame,
            } => {
                // Conservative preview watchdog: native silence is not proof of
                // static pixels. Do not resume the session after prolonged silence.
                ensure!(
                    last_frame.elapsed() < Duration::from_secs(15),
                    "WGC freshness deadline exceeded; authorize again"
                );
                session.check_health()
            }
        }
    }

    pub(super) fn capture(
        &mut self,
        display: &HostDisplay,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<Option<RgbaImage>> {
        check()?;
        display.verify()?;
        self.check_health()?;
        let observed = match self {
            Self::Snapshot => {
                let image = display
                    .monitor
                    .capture_image()
                    .map_err(|e| anyhow!("Native capture: {e}"))?;
                Some((image, display.size()))
            }
            #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
            Self::Wgc {
                session,
                last_frame,
            } => {
                let sample = session.capture(&mut check)?;
                if sample.is_some() {
                    *last_frame = Instant::now();
                }
                sample
            }
        };
        check()?;
        display.verify()?;
        self.check_health()?;
        if let Some((image, native_size)) = observed {
            ensure!(
                native_size == display.size(),
                "Captured geometry changed; authorize again"
            );
            if matches!(self, Self::Snapshot) {
                ensure!(
                    image.dimensions() == display.size(),
                    "Snapshot geometry changed"
                );
            }
            Ok(Some(image))
        } else {
            Ok(None)
        }
    }

    pub(super) fn retarget(&mut self, config: VideoConfig) -> Result<()> {
        match self {
            Self::Snapshot => {
                let _ = config;
                Ok(())
            }
            #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
            Self::Wgc { session, .. } => session.retarget((config.width(), config.height())),
        }
    }

    pub(super) fn recycle(&mut self, frame: RgbaImage) {
        match self {
            Self::Snapshot => drop(frame),
            #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
            Self::Wgc { session, .. } => session.recycle(frame),
        }
    }

    pub(super) fn stop(&mut self) -> Result<()> {
        match self {
            Self::Snapshot => Ok(()),
            #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
            Self::Wgc { session, .. } => session.stop(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_requires_same_display_name_geometry_position_and_rotation() {
        let identity = || Identity {
            id: 12,
            name: "Display A".to_owned(),
            size: (1920, 1080),
            origin: (0, 0),
            rotation: 0.0,
        };
        let original = identity();
        assert_eq!(original, identity());
        assert_ne!(
            original,
            Identity {
                id: 13,
                ..identity()
            }
        );
        assert_ne!(
            original,
            Identity {
                name: "Display B".to_owned(),
                ..identity()
            }
        );
        assert_ne!(
            original,
            Identity {
                size: (1080, 1920),
                ..identity()
            }
        );
        assert_ne!(
            original,
            Identity {
                origin: (1920, 0),
                ..identity()
            }
        );
        assert_ne!(
            original,
            Identity {
                rotation: 90.0,
                ..identity()
            }
        );
    }

    #[test]
    fn explicit_backend_never_silently_falls_back() {
        assert!(CaptureBackend::Snapshot.validate().is_ok());
        let supported = cfg!(all(target_os = "windows", feature = "wgc-preview"));
        assert_eq!(CaptureBackend::Wgc.validate().is_ok(), supported);
        assert_eq!(CaptureBackend::WgcGpu.validate().is_ok(), supported);
    }
}
