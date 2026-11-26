//! Windows-only local media diagnostic. Capture only a disposable child
//! window belonging to this executable; no desktop capture or network I/O.
use std::{
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, ensure};
use image::RgbaImage;
use intralink_core::desktop::{CodecMode, VideoConfig};
use intralink_transport::desktop::Capture;
use minifb::{Key, ScaleMode, Window, WindowOptions};

use super::codec::{Decoder, Encoder};

const TITLE: &str = "IntraLink local test pattern";
const COLORS: [u32; 4] = [0x205080, 0x40a060, 0xc06030, 0x503090];
const PERIOD: Duration = Duration::from_nanos(16_666_667);

fn dimensions(height: u16) -> Result<(usize, usize)> {
    let config = VideoConfig::new(height, 60, 75, 30).context("Use 540, 720 or 1080")?;
    Ok((config.width() as usize, config.height() as usize))
}

fn draw_background(pixels: &mut [u32], width: usize, height: usize, colors: [u32; 4]) {
    for (y, row) in pixels.chunks_exact_mut(width).enumerate() {
        row.fill(colors[2 * usize::from(y >= height / 2)]);
        row[width / 2..].fill(colors[2 * usize::from(y >= height / 2) + 1]);
    }
}

fn static_colors(final_state: bool) -> [u32; 4] {
    let mut colors = COLORS;
    colors[usize::from(final_state)] = COLORS[3];
    colors
}

fn draw_static_pattern(pixels: &mut [u32], width: usize, height: usize, final_state: bool) {
    // Large constant-color regions isolate retention from native DPI filtering.
    draw_background(pixels, width, height, static_colors(final_state));
}

fn draw_pattern(pixels: &mut [u32], width: usize, height: usize, phase: usize) {
    draw_background(pixels, width, height, COLORS);
    let block = (width / 8).max(1);
    let left = (phase * 17) % (width - block + 1);
    for y in height / 2 - 16..height / 2 + 16 {
        pixels[y * width + left..y * width + left + block].fill(0xe0d040);
    }
}

fn verify_pattern(frame: &RgbaImage, width: usize, height: usize) -> Result<()> {
    verify_colors(frame, width, height, COLORS)
}

fn verify_colors(frame: &RgbaImage, width: usize, height: usize, colors: [u32; 4]) -> Result<()> {
    super::codec::validate_source_size(frame.width(), frame.height())?;
    // Native Windows DPI can scale the logical window. Validate the actual
    // captured geometry and sample normalized quadrants, not logical pixels.
    let actual = u64::from(frame.width()) * height as u64;
    let expected = u64::from(frame.height()) * width as u64;
    ensure!(
        actual.abs_diff(expected) <= expected / 100,
        "Unexpected diagnostic window aspect ratio: {:?}",
        frame.dimensions()
    );
    let (width, height) = (frame.width() as usize, frame.height() as usize);
    for (x, y, expected) in [
        (width / 4, height / 4, colors[0]),
        (3 * width / 4, height / 4, colors[1]),
        (width / 4, 3 * height / 4, colors[2]),
        (3 * width / 4, 3 * height / 4, colors[3]),
    ] {
        let p = frame.get_pixel(x as u32, y as u32);
        let actual = u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]);
        ensure!(
            actual == expected,
            "Diagnostic pattern marker mismatch at ({x},{y}): expected {expected:06x}, got {actual:06x}"
        );
    }
    Ok(())
}

fn window(title: &str, width: usize, height: usize) -> Result<Window> {
    let mut window = Window::new(
        title,
        width,
        height,
        WindowOptions {
            borderless: true,
            title: false,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .map_err(|error| anyhow!("Diagnostic window: {error}"))?;
    window.set_target_fps(0);
    Ok(window)
}

/// The child has a hard lifetime limit even if the parent is interrupted.
pub fn pattern_source(height: u16, static_pattern: bool, geometry_pattern: bool) -> Result<()> {
    #[cfg(not(feature = "wgc-preview"))]
    ensure!(!geometry_pattern, "Geometry fixture requires wgc-preview");
    let (width, height) = dimensions(height)?;
    let mut window = window(TITLE, width, height)?;
    window.set_position(20, 20);
    let mut pixels = vec![0; width * height];
    let started = Instant::now();
    let mut phase = 0;
    let mut last_painted = None;
    #[cfg(feature = "wgc-preview")]
    let resize_request = geometry_pattern.then(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        thread::spawn(move || {
            use std::io::Read;
            let mut request = [0];
            if std::io::stdin().read_exact(&mut request).is_ok() && request == [b'R'] {
                let _ = tx.send(());
            }
        });
        rx
    });
    #[cfg(feature = "wgc-preview")]
    let mut resized = false;
    while window.is_open()
        && !window.is_key_down(Key::Escape)
        && started.elapsed() < Duration::from_secs(45)
    {
        let tick = Instant::now();
        #[cfg(feature = "wgc-preview")]
        if !resized
            && resize_request
                .as_ref()
                .is_some_and(|rx| rx.try_recv().is_ok())
        {
            intralink_d3d11::shrink_owned_window(window.get_window_handle() as usize)?;
            resized = true;
        }
        if static_pattern {
            // Two complete images, then no further submissions. The second
            // image arrives while the diagnostic consumer makes no requests.
            phase = usize::from(started.elapsed() >= Duration::from_secs(3));
        }
        if last_painted != Some(phase) {
            if static_pattern {
                draw_static_pattern(&mut pixels, width, height, phase == 1);
            } else {
                draw_pattern(&mut pixels, width, height, phase);
            }
            window
                .update_with_buffer(&pixels, width, height)
                .map_err(|error| anyhow!("Pattern presentation: {error}"))?;
            last_painted = Some(phase);
        } else {
            window.update();
        }
        if !static_pattern {
            phase = (phase + 1) % width;
        }
        thread::sleep(PERIOD.saturating_sub(tick.elapsed()));
    }
    Ok(())
}

struct Fixture(Child);

impl Drop for Fixture {
    fn drop(&mut self) {
        // The handle refers only to the diagnostic child created by this run.
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn find_fixture(child: &mut Child) -> Result<xcap::Window> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        ensure!(
            child.try_wait()?.is_none(),
            "Test pattern exited before capture"
        );
        let windows =
            xcap::Window::all().map_err(|error| anyhow!("Window enumeration: {error}"))?;
        if let Some(target) = windows.into_iter().find(|target| {
            target.pid().ok() == Some(child.id()) && target.title().ok().as_deref() == Some(TITLE)
        }) {
            return Ok(target);
        }
        thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("Own diagnostic child window was not found")
}

#[cfg(feature = "nvenc-preview")]
fn wgc_fixture(height: u16, samples: u16) -> Result<Fixture> {
    dimensions(height)?;
    ensure!((1..=180).contains(&samples), "Use 1-180 samples");
    let mut child = Fixture(
        Command::new(std::env::current_exe()?)
            .args(["desktop-test-pattern", "--height", &height.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("Start owned WGC codec fixture")?,
    );
    let _target = find_fixture(&mut child.0)?;
    Ok(child)
}

#[cfg(feature = "nvenc-preview")]
pub fn wgc_codec_check(
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    present: bool,
) -> Result<()> {
    let child = wgc_fixture(height, samples)?;
    intralink_d3d11::verify_wgc_codec(child.0.id(), TITLE, height, samples, adapter_index, present)
}

#[cfg(feature = "nvenc-preview")]
pub async fn wgc_live_check(
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    cancel_after: Option<u16>,
    isolated_decoder: bool,
    restricted_decoder: bool,
) -> Result<()> {
    ensure!(
        cancel_after.is_none_or(|count| count > 0 && count < samples),
        "Cancellation must precede the final frame"
    );
    ensure!(
        !restricted_decoder || isolated_decoder,
        "Restricted decoder requires process isolation"
    );
    let mode = if restricted_decoder {
        super::live_h264::DecoderMode::Restricted
    } else if isolated_decoder {
        super::live_h264::DecoderMode::Process
    } else {
        super::live_h264::DecoderMode::Thread
    };
    let child = wgc_fixture(height, samples)?;
    super::live_h264::run(
        child.0.id(),
        TITLE,
        height,
        samples,
        adapter_index,
        cancel_after,
        mode,
    )
    .await
}

fn metrics(label: &str, mut samples: Vec<Duration>) {
    if samples.is_empty() {
        println!("{label}: no samples");
        return;
    }
    samples.sort_unstable();
    let rank = |percent: usize| {
        samples[(samples.len() * percent).div_ceil(100) - 1].as_secs_f64() * 1000.0
    };
    println!(
        "{label}: count={}, p50_ms={:.3}, p95_ms={:.3}, p99_ms={:.3}",
        samples.len(),
        rank(50),
        rank(95),
        rank(99)
    );
}

pub fn run(height: u16, samples: u16, mode: CodecMode, wgc: bool, gpu_scale: bool) -> Result<()> {
    ensure!(!gpu_scale || wgc, "GPU scaling requires WGC");
    ensure!((1..=180).contains(&samples), "Use 1-180 samples");
    #[cfg(not(feature = "wgc-preview"))]
    ensure!(!wgc, "WGC requires a build with --features wgc-preview");
    let (width, output_height) = dimensions(height)?;
    let config = VideoConfig::new(height, 60, 75, 30)
        .context("Invalid profile")?
        .with_mode(mode);
    let mut child = Fixture(
        Command::new(std::env::current_exe()?)
            .args(["desktop-test-pattern", "--height", &height.to_string()])
            .spawn()
            .context("Start local test pattern")?,
    );
    let target = find_fixture(&mut child.0)?;
    #[cfg(feature = "wgc-preview")]
    let mut wgc_session = if wgc {
        Some(super::wgc_check::CaptureSession::start_fixture(
            child.0.id(),
            TITLE,
            gpu_scale.then_some((width as u32, output_height as u32)),
        )?)
    } else {
        None
    };
    let mut viewer = window(
        "IntraLink local decoded preview",
        width / 3,
        output_height / 3,
    )?;
    viewer.set_position((width + 40) as isize, 20);
    let mut encoder = Encoder::new(config);
    let mut decoder = Decoder::default();
    let mut payload = Vec::new();
    let mut pixels = Vec::new();
    let capacity = usize::from(samples);
    let mut capture_times = Vec::with_capacity(capacity);
    let mut encode_times = Vec::with_capacity(capacity);
    let mut decode_times = Vec::with_capacity(capacity);
    let mut present_times = Vec::with_capacity(capacity);
    let mut pipeline_times = Vec::with_capacity(capacity);
    let mut changed = 0usize;
    let mut native_samples = 0usize;
    let mut no_update = 0usize;
    let mut bytes = 0usize;
    let mut source_dimensions = None;
    let mut readback_dimensions = None;
    let started = Instant::now();
    for _ in 0..samples {
        let tick = Instant::now();
        ensure!(
            started.elapsed() < Duration::from_secs(30),
            "Diagnostic time limit reached"
        );
        ensure!(
            viewer.is_open() && !viewer.is_key_down(Key::Escape),
            "Diagnostic cancelled"
        );
        ensure!(
            child.0.try_wait()?.is_none()
                && target.pid().ok() == Some(child.0.id())
                && target.title().ok().as_deref() == Some(TITLE)
                && target.is_minimized().ok() == Some(false),
            "Diagnostic source no longer available"
        );
        let stage = Instant::now();
        let gdi_capture = || -> Result<_> {
            let image = target
                .capture_image()
                .map_err(|error| anyhow!("Local pattern capture: {error}"))?;
            let size = image.dimensions();
            Ok((image, size))
        };
        #[cfg(feature = "wgc-preview")]
        let captured = match &mut wgc_session {
            Some(session) => session.capture(|| {
                viewer.update();
                ensure!(
                    viewer.is_open() && !viewer.is_key_down(Key::Escape),
                    "Diagnostic cancelled"
                );
                ensure!(child.0.try_wait()?.is_none(), "Diagnostic source exited");
                Ok(())
            })?,
            None => Some(gdi_capture()?),
        };
        #[cfg(not(feature = "wgc-preview"))]
        let captured = Some(gdi_capture()?);
        capture_times.push(stage.elapsed());
        let Some((frame, native_size)) = captured else {
            no_update += 1;
            viewer.update();
            thread::sleep(PERIOD.saturating_sub(tick.elapsed()));
            continue;
        };
        native_samples += 1;
        verify_pattern(&frame, width, output_height)?;
        ensure!(
            source_dimensions.is_none_or(|size| size == native_size)
                && readback_dimensions.is_none_or(|size| size == frame.dimensions()),
            "Diagnostic source or readback geometry changed"
        );
        source_dimensions = Some(native_size);
        readback_dimensions = Some(frame.dimensions());
        let stage = Instant::now();
        let record = encoder.encode(&frame, payload)?;
        #[cfg(feature = "wgc-preview")]
        if let Some(session) = &mut wgc_session {
            session.recycle(frame);
        }
        encode_times.push(stage.elapsed());
        match record {
            Capture::Frame(data) => {
                let stage = Instant::now();
                decoder.decode_into(config, &data, &mut pixels)?;
                decode_times.push(stage.elapsed());
                bytes += data.len();
                changed += 1;
                payload = data;
            }
            Capture::Repeat(data) => payload = data,
            Capture::End | Capture::NoUpdate(_) => anyhow::bail!("Unexpected local encoder record"),
        }
        ensure!(!pixels.is_empty(), "Missing decoded keyframe");
        let stage = Instant::now();
        viewer
            .update_with_buffer(&pixels, width, output_height)
            .map_err(|error| anyhow!("Decoded preview: {error}"))?;
        present_times.push(stage.elapsed());
        pipeline_times.push(tick.elapsed());
        thread::sleep(PERIOD.saturating_sub(tick.elapsed()));
    }
    let elapsed = started.elapsed().as_secs_f64();
    #[cfg(feature = "wgc-preview")]
    if let Some(session) = &mut wgc_session {
        let stats = session.stats()?;
        session.stop()?;
        println!(
            "staging_allocations={}, rgba_allocations={}, gpu_processors={}, readback_bytes={}, native_updates={}, readback_frames={}",
            stats.staging_allocations,
            stats.rgba_allocations,
            stats.gpu_processors,
            stats.readback_bytes,
            stats.native_updates,
            stats.readback_frames
        );
    }
    println!(
        "capture_backend={}",
        if gpu_scale {
            "persistent_wgc_d3d11_scale"
        } else if wgc {
            "persistent_wgc"
        } else {
            "gdi_window"
        }
    );
    ensure!(
        native_samples > 0 && (samples == 1 || changed > 1),
        "No changing frames observed"
    );
    println!("native_samples={native_samples}, no_new_sample={no_update}");
    println!(
        "local_media_check,mode={mode:?},resolution={width}x{output_height},samples={samples},changed={changed},payload_bytes={bytes},elapsed_s={elapsed:.3},capture_fps={:.2},changed_fps={:.2}",
        native_samples as f64 / elapsed,
        changed as f64 / elapsed
    );
    let (source_width, source_height) = source_dimensions.context("No native frames captured")?;
    println!("source_resolution={source_width}x{source_height}");
    let (readback_width, readback_height) = readback_dimensions.context("No readback frames")?;
    println!("readback_resolution={readback_width}x{readback_height}");
    metrics("capture", capture_times);
    metrics("encode", encode_times);
    metrics("decode_changed", decode_times);
    metrics("native_buffer_submission", present_times);
    metrics("local_pipeline", pipeline_times);
    println!("No desktop capture, remote connection, or image recording performed.");
    Ok(())
}

#[cfg(any(test, feature = "wgc-preview"))]
fn verify_static_marker(
    frame: &RgbaImage,
    width: usize,
    height: usize,
    final_state: bool,
) -> Result<()> {
    // Both top quadrants must match, including restoration of the old marker.
    verify_colors(frame, width, height, static_colors(final_state))
}

#[cfg(feature = "wgc-preview")]
pub fn static_check(height: u16, gpu_scale: bool) -> Result<()> {
    let (width, output_height) = dimensions(height)?;
    let mut child = Fixture(
        Command::new(std::env::current_exe()?)
            .args([
                "desktop-test-pattern",
                "--height",
                &height.to_string(),
                "--static-pattern",
            ])
            .spawn()
            .context("Start static test pattern")?,
    );
    let target = find_fixture(&mut child.0)?;
    let mut session = super::wgc_check::CaptureSession::start_fixture(
        child.0.id(),
        TITLE,
        gpu_scale.then_some((width as u32, output_height as u32)),
    )?;
    let mut check = || -> Result<()> {
        ensure!(
            child.0.try_wait()?.is_none()
                && target.pid().ok() == Some(child.0.id())
                && target.title().ok().as_deref() == Some(TITLE)
                && target.is_minimized().ok() == Some(false),
            "Static fixture unavailable"
        );
        Ok(())
    };
    let (first, native_size) = session
        .capture(&mut check)?
        .context("Missing initial frame")?;
    verify_static_marker(&first, width, output_height, false).context("Initial static frame")?;
    session.recycle(first);
    let before = session.stats()?;
    // Deliberately make no capture requests across the final source change.
    let idle = Instant::now();
    while idle.elapsed() < Duration::from_secs(5) {
        check()?;
        thread::sleep(Duration::from_millis(50));
    }
    let after_idle = session.stats()?;
    ensure!(
        after_idle.readback_frames == before.readback_frames
            && after_idle.readback_bytes == before.readback_bytes,
        "CPU readback occurred without a consumer request"
    );
    ensure!(
        after_idle.native_updates > before.native_updates,
        "Final source update was not observed during idle"
    );
    let (last, size) = session
        .capture(&mut check)?
        .context("Final update was lost")?;
    ensure!(size == native_size, "Static source geometry changed");
    verify_static_marker(&last, width, output_height, true)
        .context("Final static frame after idle")?;
    session.recycle(last);
    let cancelled = session.capture(|| anyhow::bail!("Local fixture cancellation"));
    ensure!(
        matches!(cancelled, Err(error) if error.to_string() == "Local fixture cancellation"),
        "Local cancellation did not take priority"
    );
    let stats = session.stats()?;
    ensure!(
        stats.readback_frames == 2 && stats.staging_allocations == 1 && stats.rgba_allocations == 1,
        "Static fixture resource reuse or cancellation failed"
    );
    // Only close the disposable child created by this diagnostic. Wait for
    // the actual native Closed event rather than inferring it from silence.
    child.0.kill().context("Close static fixture")?;
    child.0.wait().context("Join static fixture")?;
    let closing = Instant::now();
    while !session.source_closed()? {
        ensure!(
            closing.elapsed() < Duration::from_secs(2),
            "WGC source closure deadline exceeded"
        );
        thread::sleep(Duration::from_millis(8));
    }
    ensure!(
        session.capture(|| Ok(())).is_err(),
        "Closed WGC source accepted a readback"
    );
    session.stop()?;
    ensure!(
        session.capture(|| Ok(())).is_err(),
        "Stopped WGC session accepted a readback"
    );
    println!("local_cancel=passed, native_close=passed, readback_after_stop=rejected");
    println!(
        "static_check: passed, gpu_scale={gpu_scale}, native_updates={}, readback_frames={}, staging_allocations={}, rgba_allocations={}, idle_readback_bytes=0",
        stats.native_updates,
        stats.readback_frames,
        stats.staging_allocations,
        stats.rgba_allocations
    );
    Ok(())
}

/// Exercise real WGC pool recreation by resizing only our disposable window.
#[cfg(feature = "wgc-preview")]
pub fn geometry_check() -> Result<()> {
    use std::io::Write;
    intralink_d3d11::verify_native_boundary()?;
    println!("native_device_context_texture_and_geometry_rejection=passed");
    let mut child = Fixture(
        Command::new(std::env::current_exe()?)
            .args([
                "desktop-test-pattern",
                "--height",
                "540",
                "--geometry-pattern",
            ])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("Start geometry fixture")?,
    );
    let _target = find_fixture(&mut child.0)?;
    let mut session = super::wgc_check::CaptureSession::start_fixture(child.0.id(), TITLE, None)?;
    let (first, original) = session.capture(|| Ok(()))?.context("Missing first frame")?;
    verify_pattern(&first, 960, 540)?;
    session.recycle(first);
    let before = session.stats()?;
    child
        .0
        .stdin
        .take()
        .context("Missing fixture pipe")?
        .write_all(b"R")?;
    let started = Instant::now();
    let failure = loop {
        ensure!(child.0.try_wait()?.is_none(), "Geometry fixture exited");
        if let Some(failure) = session.failure()? {
            break failure;
        }
        ensure!(
            started.elapsed() < Duration::from_secs(5),
            "Native geometry failure not observed"
        );
        thread::sleep(Duration::from_millis(8));
    };
    ensure!(
        failure.contains("WGC source geometry or device changed"),
        "Unexpected native failure: {failure}"
    );
    let after = session.stats()?;
    ensure!(
        after.readback_frames == before.readback_frames,
        "Unexpected readback after resize"
    );
    ensure!(
        session.capture(|| Ok(())).is_err(),
        "Changed source published pixels"
    );
    let stop_result = session.stop();
    ensure!(
        stop_result.is_err(),
        "Native callback failure not propagated at join"
    );
    ensure!(
        session.capture(|| Ok(())).is_err(),
        "Stopped source published pixels"
    );
    println!(
        "geometry_check: passed, original={}x{}, failure={failure}, elapsed_ms={}, readback_frames={}",
        original.0,
        original.1,
        started.elapsed().as_millis(),
        after.readback_frames
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_markers_distinguish_final_update_from_stale_frame() {
        let (w, h) = dimensions(540).unwrap();
        let mut pixels = vec![0; w * h];
        for final_state in [false, true] {
            draw_static_pattern(&mut pixels, w, h, final_state);
            let rgba = pixels
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255])
                .collect();
            let image = RgbaImage::from_raw(w as u32, h as u32, rgba).unwrap();
            verify_static_marker(&image, w, h, final_state).unwrap();
            assert!(verify_static_marker(&image, w, h, !final_state).is_err());
            assert!(verify_pattern(&image, w, h).is_err());
            let scaled =
                image::imageops::resize(&image, 1656, 933, image::imageops::FilterType::Triangle);
            verify_static_marker(&scaled, w, h, final_state).unwrap();
            assert!(verify_static_marker(&scaled, w, h, !final_state).is_err());
        }
    }

    #[test]
    fn fixture_markers_survive_animation_and_reject_wrong_content() {
        let (width, height) = dimensions(540).unwrap();
        let mut pixels = vec![0; width * height];
        for phase in [0, 1, 80, width - 1] {
            draw_pattern(&mut pixels, width, height, phase);
            let rgba = pixels
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255])
                .collect();
            let image = RgbaImage::from_raw(width as u32, height as u32, rgba).unwrap();
            verify_pattern(&image, width, height).unwrap();
            let scaled =
                image::imageops::resize(&image, 1641, 924, image::imageops::FilterType::Nearest);
            verify_pattern(&scaled, width, height).unwrap();
        }
        assert!(
            verify_pattern(&RgbaImage::new(width as u32, height as u32), width, height).is_err()
        );
        assert!(verify_pattern(&RgbaImage::new(1, 1), width, height).is_err());
        assert!(verify_pattern(&RgbaImage::new(0, 0), width, height).is_err());
    }
}
