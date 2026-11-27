//! Owned generated window shared by local capture diagnostics.
//! Never enumerate or capture arbitrary application windows as a source.
use anyhow::{Context, Result, anyhow, ensure};
use image::RgbaImage;
use intralink_core::desktop::VideoConfig;
use minifb::{Key, ScaleMode, Window, WindowOptions};
use std::{
    process::Child,
    thread,
    time::{Duration, Instant},
};

pub(super) const TITLE: &str = "IntraLink local test pattern";
const COLORS: [u32; 4] = [0x205080, 0x40a060, 0xc06030, 0x503090];
pub(super) const PERIOD: Duration = Duration::from_nanos(16_666_667);

pub(super) fn dimensions(height: u16) -> Result<(usize, usize)> {
    let config = VideoConfig::new(height, 60, 75, 30).context("Use 540, 720 or 1080")?;
    Ok((config.width() as usize, config.height() as usize))
}

fn draw_background(pixels: &mut [u32], width: usize, height: usize, colors: [u32; 4]) {
    for (y, row) in pixels.chunks_exact_mut(width).enumerate() {
        row.fill(colors[2 * usize::from(y >= height / 2)]);
        row[width / 2..].fill(colors[2 * usize::from(y >= height / 2) + 1]);
    }
}

pub(super) fn static_colors(final_state: bool) -> [u32; 4] {
    let mut colors = COLORS;
    colors[usize::from(final_state)] = COLORS[3];
    colors
}

pub(super) fn draw_static_pattern(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    final_state: bool,
) {
    // Large constant-color regions isolate retention from native DPI filtering.
    draw_background(pixels, width, height, static_colors(final_state));
}

pub(super) fn draw_pattern(pixels: &mut [u32], width: usize, height: usize, phase: usize) {
    draw_background(pixels, width, height, COLORS);
    let block = (width / 8).max(1);
    let left = (phase * 17) % (width - block + 1);
    for y in height / 2 - 16..height / 2 + 16 {
        pixels[y * width + left..y * width + left + block].fill(0xe0d040);
    }
}

pub(super) fn verify_pattern(frame: &RgbaImage, width: usize, height: usize) -> Result<()> {
    verify_colors(frame, width, height, COLORS)
}

pub(super) fn verify_colors(
    frame: &RgbaImage,
    width: usize,
    height: usize,
    colors: [u32; 4],
) -> Result<()> {
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

pub(super) fn window(title: &str, width: usize, height: usize) -> Result<Window> {
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

#[derive(clap::Args)]
pub struct PatternArgs {
    #[arg(long)]
    height: u16,
    #[arg(long)]
    static_pattern: bool,
    #[arg(long, conflicts_with = "static_pattern")]
    geometry_pattern: bool,
    /// Calm labeled UI animation for local capture/encode diagnostics.
    #[arg(long, conflicts_with_all = ["static_pattern", "geometry_pattern"])]
    visual_pattern: bool,
}

impl PatternArgs {
    pub fn run(self) -> Result<()> {
        pattern_source(
            self.height,
            self.static_pattern,
            self.geometry_pattern,
            self.visual_pattern,
        )
    }
}

/// The child has a hard lifetime limit even if the parent is interrupted.
fn pattern_source(
    height: u16,
    static_pattern: bool,
    geometry_pattern: bool,
    visual_pattern: bool,
) -> Result<()> {
    #[cfg(not(all(target_os = "windows", feature = "wgc-preview")))]
    ensure!(!geometry_pattern, "Geometry fixture requires wgc-preview");
    let (width, height) = dimensions(height)?;
    let mut window = window(TITLE, width, height)?;
    window.set_position(20, 20);
    let mut pixels = vec![0; width * height];
    let started = Instant::now();
    let mut phase = 0;
    let mut last_painted = None;
    #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
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
    #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
    let mut resized = false;
    while window.is_open()
        && !window.is_key_down(Key::Escape)
        && started.elapsed() < Duration::from_secs(45)
    {
        let tick = Instant::now();
        #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
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
            if visual_pattern {
                super::visual_fixture::draw(&mut pixels, width, height, phase);
            } else if static_pattern {
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

pub(super) struct Fixture(pub(super) Child);

impl Drop for Fixture {
    fn drop(&mut self) {
        // The handle refers only to the diagnostic child created by this run.
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

pub(super) fn find_fixture(child: &mut Child) -> Result<xcap::Window> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        ensure!(
            child.try_wait()?.is_none(),
            "Test pattern exited before capture"
        );
        let windows =
            xcap::Window::all().map_err(|error| anyhow!("Window enumeration: {error}"))?;
        let mut candidates = windows.into_iter().filter(|target| {
            target.pid().ok() == Some(child.id()) && target.title().ok().as_deref() == Some(TITLE)
        });
        if let Some(target) = candidates.next() {
            ensure!(
                candidates.next().is_none(),
                "Ambiguous owned diagnostic window"
            );
            return Ok(target);
        }
        thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("Own diagnostic child window was not found")
}
