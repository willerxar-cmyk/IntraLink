//! Calm, recognizable local diagnostic scene. High-entropy stress fixtures
//! remain in memory; never present full-window alternating noise.
#[cfg(any(test, all(target_os = "windows", feature = "wgc-preview")))]
use anyhow::{Result, ensure};
#[cfg(any(test, all(target_os = "windows", feature = "wgc-preview")))]
use image::RgbaImage;
#[cfg(any(test, all(target_os = "windows", feature = "wgc-preview")))]
use intralink_core::desktop::VideoConfig;

const LOGICAL_WIDTH: usize = 960;
const LOGICAL_HEIGHT: usize = 540;
const BACKGROUND: u32 = 0xe8edf2;
const INK: u32 = 0x253448;
const PAPER: u32 = 0xf7f9fb;
const MUTED: u32 = 0xb5c1cd;
const ACCENT: u32 = 0x52758c;
const MARKERS: [u32; 4] = [0xb86f70, 0x648e78, 0x6087b4, 0x9fa6ad];
const LABELS: [&str; 4] = ["RED", "GREEN", "BLUE", "GRAY"];

struct Canvas<'a> {
    pixels: &'a mut [u32],
    size: (usize, usize),
}

impl Canvas<'_> {
    fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        let (width, height) = self.size;
        let left = x * width / LOGICAL_WIDTH;
        let right = (x + w).min(LOGICAL_WIDTH) * width / LOGICAL_WIDTH;
        let top = y * height / LOGICAL_HEIGHT;
        let bottom = (y + h).min(LOGICAL_HEIGHT) * height / LOGICAL_HEIGHT;
        for row in self.pixels.chunks_exact_mut(width).take(bottom).skip(top) {
            row[left..right].fill(color);
        }
    }

    fn label(&mut self, x: usize, y: usize, text: &str, scale: usize, color: u32) {
        for (i, c) in text.chars().enumerate() {
            for (row, mask) in super::overview::glyph(c).iter().enumerate() {
                for col in 0..5 {
                    if mask & (1 << (4 - col)) != 0 {
                        self.rect(
                            x + (i * 6 + col) * scale,
                            y + row * scale,
                            scale,
                            scale,
                            color,
                        );
                    }
                }
            }
        }
    }
}

/// The header, color cards and document remain stable. Only the neutral
/// chart scrolls and its cursor moves; no full-canvas color inversion.
pub(super) fn draw(pixels: &mut [u32], width: usize, height: usize, phase: usize) {
    let mut c = Canvas {
        pixels,
        size: (width, height),
    };
    c.pixels.fill(BACKGROUND);
    c.rect(0, 0, 960, 68, INK);
    c.label(24, 12, "INTRALINK - LOCAL IMAGE TEST", 3, PAPER);
    c.label(24, 42, "GENERATED CONTENT / ESC TO CLOSE", 2, 0xc4d1df);
    for (i, color) in MARKERS.into_iter().enumerate() {
        let x = 24 + i * 240;
        c.rect(x, 84, 192, 94, PAPER);
        c.rect(x + 24, 103, 144, 55, color);
        c.label(x + 12, 163, LABELS[i], 1, INK);
    }
    c.rect(24, 194, 336, 300, PAPER);
    c.label(40, 210, "DOCUMENT PREVIEW", 2, INK);
    c.rect(40, 239, 240, 5, ACCENT);
    for row in 0..12 {
        let width = 224 - (row % 4) * 21;
        c.rect(40, 258 + row * 15, width, 3, MUTED);
    }
    c.label(40, 451, "FIXED TEXT AND COLORS", 1, INK);
    c.rect(376, 194, 560, 300, PAPER);
    c.label(392, 210, "MOVING DETAIL / FRAME CHECK", 2, INK);
    // Muted fine detail resembles a chart/texture rather than random color
    // corruption. Its movement is continuous and localized to one panel.
    let (w, h) = c.size;
    let x0 = 392 * w / LOGICAL_WIDTH;
    let x1 = 920 * w / LOGICAL_WIDTH;
    let y0 = 245 * h / LOGICAL_HEIGHT;
    let y1 = 449 * h / LOGICAL_HEIGHT;
    let phase = phase % 1920;
    for (y, row) in c.pixels.chunks_exact_mut(w).enumerate().take(y1).skip(y0) {
        let gy = (y - y0) * LOGICAL_HEIGHT / h;
        for (x, pixel) in row.iter_mut().enumerate().take(x1).skip(x0) {
            let gx = (x - x0) * LOGICAL_WIDTH / w + phase;
            let wave = (gx * 3 + gy * 2) % 64;
            let shade = 178 + wave.min(63 - wave) as u32;
            let grid = usize::from(gx.is_multiple_of(44) || gy.is_multiple_of(34)) as u32 * 16;
            let v = shade.saturating_sub(grid);
            *pixel = ((v - 8) << 16) | ((v + 3) << 8) | (v + 10);
        }
    }
    let cursor = 392 + phase % 500;
    c.rect(cursor, 245, 2, 204, ACCENT);
    c.rect(392, 464, 528, 6, MUTED);
    c.rect(392, 464, 12 + phase % 516, 6, ACCENT);
    c.label(24, 511, "LOCAL DIAGNOSTIC - NOT A REMOTE DESKTOP", 2, INK);
}

#[cfg(any(test, all(target_os = "windows", feature = "wgc-preview")))]
fn marker_positions(width: usize, height: usize) -> [(usize, usize); 4] {
    std::array::from_fn(|i| ((2 * i + 1) * width / 8, height / 4))
}

#[cfg(any(test, all(target_os = "windows", feature = "wgc-preview")))]
fn near(actual: u32, expected: u32, tolerance: u8) -> bool {
    [0, 8, 16]
        .into_iter()
        .all(|shift| ((actual >> shift) as u8).abs_diff((expected >> shift) as u8) <= tolerance)
}

#[cfg(any(test, all(target_os = "windows", feature = "wgc-preview")))]
pub(super) fn verify_native(frame: &RgbaImage) -> Result<()> {
    let (width, height) = (frame.width() as usize, frame.height() as usize);
    ensure!(
        width > 0 && height > 0 && (width * 9).abs_diff(height * 16) <= height / 4,
        "Visual source geometry changed"
    );
    for ((x, y), expected) in marker_positions(width, height).into_iter().zip(MARKERS) {
        let p = frame.get_pixel(x as u32, y as u32);
        let actual = (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2]);
        ensure!(
            near(actual, expected, 3),
            "Native visual color card mismatch"
        );
    }
    Ok(())
}

#[cfg(any(test, all(target_os = "windows", feature = "wgc-preview")))]
pub(super) fn verify_decoded(config: VideoConfig, pixels: &[u32]) -> Result<()> {
    let (width, height) = (config.width() as usize, config.height() as usize);
    ensure!(
        pixels.len() == width * height,
        "Visual decoded geometry changed"
    );
    for ((x, y), expected) in marker_positions(width, height).into_iter().zip(MARKERS) {
        ensure!(
            near(pixels[y * width + x], expected, 8),
            "Decoded visual color card mismatch"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_cards_are_stable_and_animation_is_localized() -> Result<()> {
        for h in [540, 720, 1080] {
            let config = VideoConfig::new(h, 60, 75, 30).unwrap();
            let (w, h) = (config.width() as usize, config.height() as usize);
            let mut first = vec![0; w * h];
            let mut second = first.clone();
            draw(&mut first, w, h, 0);
            draw(&mut second, w, h, 61);
            verify_decoded(config, &first)?;
            verify_decoded(config, &second)?;
            assert_eq!(&first[..h / 3 * w], &second[..h / 3 * w]);
            let changed = first.iter().zip(&second).filter(|(a, b)| a != b).count();
            assert!(changed > 0 && changed < w * h / 3);
            let swapped: Vec<u32> = first
                .iter()
                .map(|p| ((p & 255) << 16) | (p & 0xff00) | ((p >> 16) & 255))
                .collect();
            assert!(verify_decoded(config, &swapped).is_err());
            let rgba = first
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255])
                .collect();
            verify_native(&RgbaImage::from_raw(w as u32, h as u32, rgba).unwrap())?;
        }
        Ok(())
    }
}
