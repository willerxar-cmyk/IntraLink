//! Local automatic Host selection. Only a disposable generated window is
//! captured here; the selected real display is never read by calibration.
use std::{
    process::Command,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, ensure};
use image::RgbaImage;
use intralink_core::{
    adaptive::config_at_height,
    desktop::{CodecMode, VideoConfig},
    host_calibration::{CapturePath, HostMeasurement, HostProfile, recommend},
};
use intralink_transport::desktop::Capture;

use super::{
    capture_fixture::{Fixture, TITLE, dimensions, find_fixture, verify_pattern},
    codec::{Decoder, Encoder, validate_source_size},
};

const SAMPLES: usize = 6;
const DEADLINE: Duration = Duration::from_secs(35);

enum Probe {
    Snapshot,
    #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
    Wgc(Box<super::wgc_check::CaptureSession>),
}

impl Probe {
    fn start(path: CapturePath, pid: u32, config: VideoConfig) -> Result<Self> {
        #[cfg(not(all(target_os = "windows", feature = "wgc-preview")))]
        let _ = (pid, config);
        match path {
            CapturePath::Snapshot => Ok(Self::Snapshot),
            #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
            CapturePath::Wgc | CapturePath::WgcGpu => {
                let target =
                    (path == CapturePath::WgcGpu).then_some((config.width(), config.height()));
                Ok(Self::Wgc(Box::new(
                    super::wgc_check::CaptureSession::start_fixture(pid, TITLE, target)?,
                )))
            }
            #[cfg(not(all(target_os = "windows", feature = "wgc-preview")))]
            CapturePath::Wgc | CapturePath::WgcGpu => {
                anyhow::bail!("WGC is not compiled for this platform")
            }
        }
    }

    fn capture(
        &mut self,
        target: &xcap::Window,
        check: &mut impl FnMut() -> Result<()>,
    ) -> Result<Option<(RgbaImage, (u32, u32))>> {
        check()?;
        let sample = match self {
            Self::Snapshot => {
                let image = target
                    .capture_image()
                    .map_err(|error| anyhow!("Owned pattern capture: {error}"))?;
                let size = image.dimensions();
                Some((image, size))
            }
            #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
            Self::Wgc(session) => session.capture(&mut *check)?,
        };
        check()?;
        Ok(sample)
    }

    fn recycle(&mut self, image: RgbaImage) {
        match self {
            Self::Snapshot => drop(image),
            #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
            Self::Wgc(session) => session.recycle(image),
        }
    }

    fn stop(&mut self) -> Result<()> {
        match self {
            Self::Snapshot => Ok(()),
            #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
            Self::Wgc(session) => session.stop(),
        }
    }
}

fn allowed_modes(ceiling: CodecMode) -> impl Iterator<Item = CodecMode> {
    [CodecMode::Jpeg, CodecMode::Tiles, CodecMode::Compact]
        .into_iter()
        .filter(move |mode| *mode as u8 <= ceiling as u8)
}

/// A locally generated, bounded high-frequency region plus large desktop
/// gradients and a stable central color marker. Never use user pixels.
fn dense_fixture(size: (u32, u32)) -> Result<RgbaImage> {
    let (width, height) = size;
    validate_source_size(width, height)?;
    Ok(RgbaImage::from_fn(width, height, |x, y| {
        if marker(x, y, size) {
            return image::Rgba([70, 120, 180, 255]);
        }
        let (tx, ty) = if y < height / 4 {
            (x, y)
        } else {
            (x / 8, y / 8)
        };
        let texture = ((tx * 17 + ty * 29) ^ (tx * ty)) as u8;
        image::Rgba([
            (x / 8) as u8 ^ texture,
            (y / 4) as u8 ^ (texture / 2),
            (x / 16 + y / 16) as u8,
            255,
        ])
    }))
}

fn marker(x: u32, y: u32, (width, height): (u32, u32)) -> bool {
    x.abs_diff(width / 2) < (width / 8).max(1) && y.abs_diff(height / 2) < (height / 8).max(1)
}

fn change_fixture(frame: &mut RgbaImage) {
    let size = frame.dimensions();
    for (x, y, pixel) in frame.enumerate_pixels_mut() {
        if !marker(x, y, size) {
            pixel[0] ^= 64;
            pixel[1] ^= 128;
        }
    }
}

fn verify_marker(config: VideoConfig, pixels: &[u32]) -> Result<()> {
    let (width, height) = (config.width() as usize, config.height() as usize);
    ensure!(
        pixels.len() == width * height,
        "Calibration decoded geometry mismatch"
    );
    let actual = pixels[(height / 2) * width + width / 2];
    for (shift, expected) in [(16, 70u8), (8, 120), (0, 180)] {
        ensure!(
            ((actual >> shift) as u8).abs_diff(expected) <= 8,
            "Calibration encoded marker mismatch"
        );
    }
    Ok(())
}

/// Include actual resize/compare/encode of changing dense data in every
/// software mode the negotiated Host may select, not just cheap tile updates.
fn dense_encode_cost(
    config: VideoConfig,
    size: (u32, u32),
    mut check: impl FnMut() -> Result<()>,
) -> Result<Duration> {
    check()?;
    let mut frame = dense_fixture(size)?;
    let mut worst = Duration::ZERO;
    for mode in allowed_modes(config.mode()) {
        let config = config.with_mode(mode);
        let mut encoder = Encoder::new(config);
        let mut decoder = Decoder::default();
        let mut payload = Vec::new();
        let mut pixels = Vec::new();
        for index in 0..=SAMPLES {
            check()?;
            change_fixture(&mut frame);
            let start = Instant::now();
            let encoded = encoder.encode(&frame, payload)?;
            let elapsed = start.elapsed();
            let Capture::Frame(data) = encoded else {
                anyhow::bail!("Changing calibration fixture did not encode a frame")
            };
            if index != 0 {
                worst = worst.max(elapsed);
            }
            decoder.decode_into(config, &data, &mut pixels)?;
            verify_marker(config, &pixels)?;
            payload = data;
        }
    }
    ensure!(!worst.is_zero(), "Local timer did not measure encoding");
    check()?;
    Ok(worst)
}

fn check_fixture(child: &mut Fixture, target: &xcap::Window, started: Instant) -> Result<()> {
    ensure!(
        started.elapsed() < DEADLINE,
        "Host calibration deadline reached"
    );
    ensure!(
        child.0.try_wait()?.is_none()
            && target.pid().ok() == Some(child.0.id())
            && target.title().ok().as_deref() == Some(TITLE)
            && target.is_minimized().ok() == Some(false),
        "Owned calibration window closed, minimized, changed or canceled"
    );
    Ok(())
}

struct CandidateSample {
    capture: Duration,
    encode: Duration,
    readback_size: (u32, u32),
}

fn sample_candidate(
    probe: &mut Probe,
    target: &xcap::Window,
    logical: (usize, usize),
    config: VideoConfig,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Option<CandidateSample>> {
    let mut encoder = Encoder::new(config);
    let mut decoder = Decoder::default();
    let mut payload = Vec::new();
    let mut pixels = Vec::new();
    let mut capture_worst = Duration::ZERO;
    let mut encode_worst = Duration::ZERO;
    let mut geometry = None;
    let mut changed = 0;
    for index in 0..=SAMPLES {
        check()?;
        let began = Instant::now();
        let (image, native_size) = loop {
            match probe.capture(target, check) {
                Ok(Some(sample)) => break sample,
                Ok(None) => {
                    if began.elapsed() >= Duration::from_secs(2) {
                        println!("candidate rejected: no fresh owned-window sample");
                        return Ok(None);
                    }
                }
                Err(error) => {
                    // Cancellation or source identity failure is fatal to the
                    // entire preflight, not a reason to try another backend.
                    check()?;
                    println!("candidate rejected: {error:#}");
                    return Ok(None);
                }
            }
        };
        let captured = began.elapsed();
        // A marker or identity failure is not a valid capability observation.
        verify_pattern(&image, logical.0, logical.1)?;
        let observed = (native_size, image.dimensions());
        ensure!(
            geometry.is_none_or(|prior| prior == observed),
            "Calibration source or readback geometry changed"
        );
        geometry = Some(observed);
        let start = Instant::now();
        let encoded = encoder.encode(&image, payload)?;
        let encoded_time = start.elapsed();
        probe.recycle(image);
        match encoded {
            Capture::Frame(data) => {
                decoder.decode_into(config, &data, &mut pixels)?;
                payload = data;
                if index != 0 {
                    changed += 1;
                }
            }
            Capture::Repeat(data) => payload = data,
            _ => anyhow::bail!("Unexpected local calibration record"),
        }
        if index != 0 {
            capture_worst = capture_worst.max(captured);
            encode_worst = encode_worst.max(encoded_time);
        }
    }
    check()?;
    let (native_size, readback_size) = geometry.context("No calibration sample")?;
    ensure!(
        changed > 0 && !capture_worst.is_zero() && !encode_worst.is_zero(),
        "No changing timed calibration samples"
    );
    println!(
        "source={}x{}, readback={}x{}, samples={SAMPLES}, changed={changed}",
        native_size.0, native_size.1, readback_size.0, readback_size.1
    );
    Ok(Some(CandidateSample {
        capture: capture_worst,
        encode: encode_worst,
        readback_size,
    }))
}

pub(super) fn select(ceiling: VideoConfig) -> Result<HostProfile> {
    super::require_graphical_environment()?;
    println!(
        "Local Host calibration: generated window only; close it or press ESC to cancel. No monitor capture or network."
    );
    let height = ceiling.height() as u16;
    let logical = dimensions(height)?;
    let started = Instant::now();
    let mut child = Fixture(
        Command::new(std::env::current_exe()?)
            .args(["desktop-test-pattern", "--height", &height.to_string()])
            .spawn()
            .context("Start owned Host calibration pattern")?,
    );
    let target = find_fixture(&mut child.0)?;
    let pid = child.0.id();
    let paths: &[CapturePath] = if cfg!(all(target_os = "windows", feature = "wgc-preview")) {
        &[CapturePath::Snapshot, CapturePath::Wgc, CapturePath::WgcGpu]
    } else {
        &[CapturePath::Snapshot]
    };
    let mut measurements = Vec::with_capacity(9);
    let mut check = || check_fixture(&mut child, &target, started);
    for height in [1080, 720, 540] {
        let Some(config) = config_at_height(ceiling, height) else {
            continue;
        };
        for &path in paths {
            check()?;
            println!("Host candidate: {path:?} {height}p");
            let mut probe = match Probe::start(path, pid, config) {
                Ok(probe) => probe,
                Err(error) => {
                    check()?;
                    println!("candidate rejected before sharing: {error:#}");
                    continue;
                }
            };
            let result = sample_candidate(&mut probe, &target, logical, config, &mut check);
            // Explicitly stop native capture before trying another path.
            probe.stop()?;
            let Some(CandidateSample {
                capture,
                encode: observed_encode,
                readback_size: size,
            }) = result?
            else {
                continue;
            };
            let dense = dense_encode_cost(config, size, &mut check)?;
            let encode = observed_encode.max(dense);
            println!(
                "host_measurement, backend={path:?}, height={height}, capture_worst_ms={:.3}, observed_encode_worst_ms={:.3}, dense_encode_worst_ms={:.3}",
                capture.as_secs_f64() * 1000.0,
                observed_encode.as_secs_f64() * 1000.0,
                dense.as_secs_f64() * 1000.0
            );
            measurements.push(HostMeasurement {
                path,
                height,
                capture,
                encode,
            });
        }
        if recommend(ceiling, &measurements)
            .is_some_and(|profile| profile.config.fps() >= ceiling.fps().min(30))
        {
            break;
        }
    }
    check()?;
    let selected = recommend(ceiling, &measurements)
        .context("No measured Host capture/encode profile is usable")?;
    println!(
        "selected_host_backend={:?}, height={}, fps={}, codec={:?}, quality={}, duration_s={}",
        selected.path,
        selected.config.height(),
        selected.config.fps(),
        selected.config.mode(),
        selected.config.quality(),
        selected.config.seconds()
    );
    println!(
        "Generated-window and synthetic encoder measurements only; real display, network and sustained FPS are unqualified."
    );
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_dense_fixture_uses_production_codecs_and_keeps_marker() -> Result<()> {
        let config = VideoConfig::new(540, 60, 90, 30)
            .unwrap()
            .with_mode(CodecMode::Compact);
        let worst = dense_encode_cost(config, (config.width(), config.height()), || Ok(()))?;
        assert!(!worst.is_zero());
        Ok(())
    }

    #[test]
    fn cancellation_prevents_dense_encoding() {
        let config = VideoConfig::new(540, 60, 75, 30).unwrap();
        assert!(
            dense_encode_cost(config, (config.width(), config.height()), || anyhow::bail!(
                "cancel"
            ))
            .is_err()
        );
    }
}
