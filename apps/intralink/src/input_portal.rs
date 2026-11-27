//! Bounded local visual transition for the spatial pad. No effect state
//! influences pointer coordinates, input authority or the wire.
pub(super) const DURATION_MS: u128 = 220;
const RADIUS: isize = 44;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Full,
    Reduced,
    Off,
}

impl Mode {
    pub(super) fn next(self) -> Self {
        match self {
            Self::Full => Self::Reduced,
            Self::Reduced => Self::Off,
            Self::Off => Self::Full,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Full => "FULL",
            Self::Reduced => "REDUCED",
            Self::Off => "OFF",
        }
    }

    fn settings(self) -> Option<(isize, f32)> {
        match self {
            Self::Full => Some((RADIUS, 1.0)),
            Self::Reduced => Some((26, 0.4)),
            Self::Off => None,
        }
    }
}

struct Region {
    size: (usize, usize),
    left: usize,
    top: usize,
    width: usize,
    height: usize,
}

/// Restore only the affected patch between frames. The underlying pad is
/// rendered anew only when its layout, connection or focus state changes.
#[derive(Default)]
pub(super) struct Patch {
    region: Option<Region>,
    saved: Vec<u32>,
}

impl Patch {
    pub(super) fn restore(&mut self, pixels: &mut [u32], size: (usize, usize)) -> bool {
        let Some(region) = self.region.take() else {
            return false;
        };
        if region.size != size || size.0.checked_mul(size.1) != Some(pixels.len()) {
            return false;
        }
        for row in 0..region.height {
            let start = (region.top + row) * size.0 + region.left;
            let saved = row * region.width;
            pixels[start..start + region.width]
                .copy_from_slice(&self.saved[saved..saved + region.width]);
        }
        true
    }

    pub(super) fn draw(
        &mut self,
        pixels: &mut [u32],
        size: (usize, usize),
        at: (usize, usize),
        progress: f32,
        confirmed: bool,
        mode: Mode,
    ) {
        self.restore(pixels, size);
        let (width, height) = size;
        let Some((limit, amplitude)) = mode.settings() else {
            return;
        };
        if width.checked_mul(height) != Some(pixels.len())
            || at.0 >= width
            || at.1 >= height
            || !progress.is_finite()
            || !(0.0..1.0).contains(&progress)
            || progress == 0.0
        {
            return;
        }
        let (cx, cy) = (at.0 as isize, at.1 as isize);
        let envelope = (progress * std::f32::consts::PI).sin();
        let radius = (limit as f32 * (0.55 + 0.45 * envelope)) as isize;
        let strength = amplitude * envelope * if confirmed { 1.0 } else { 0.55 };
        let left = at.0.saturating_sub(limit as usize);
        let top = at.1.saturating_sub(limit as usize);
        let right = at.0.saturating_add(limit as usize).min(width - 1);
        let bottom = at.1.saturating_add(limit as usize).min(height - 1);
        let patch_width = right - left + 1;
        let patch_height = bottom - top + 1;
        self.saved.clear();
        for y in top..=bottom {
            self.saved
                .extend_from_slice(&pixels[y * width + left..=y * width + right]);
        }
        self.region = Some(Region {
            size,
            left,
            top,
            width: patch_width,
            height: patch_height,
        });
        let accent = if confirmed {
            [98.0, 224.0, 190.0]
        } else {
            [100.0, 160.0, 235.0]
        };
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let x = cx + dx;
                let y = cy + dy;
                if x < 0 || y < 0 || x >= width as isize || y >= height as isize {
                    continue;
                }
                let r = ((dx * dx + dy * dy) as f32).sqrt() / radius as f32;
                if r >= 1.0 {
                    continue;
                }
                let twist = (1.0 - r).powi(2) * strength * 1.35;
                let (s, c) = twist.sin_cos();
                let pull = 1.0 + 0.35 * (1.0 - r) * strength;
                let sample = |offset: f32, center: isize, lower: usize, upper: usize| {
                    (center + offset.round() as isize).clamp(lower as isize, upper as isize)
                        as usize
                        - lower
                };
                let sx = sample((dx as f32 * c - dy as f32 * s) * pull, cx, left, right);
                let sy = sample((dx as f32 * s + dy as f32 * c) * pull, cy, top, bottom);
                let color = self.saved[sy * patch_width + sx];
                let ring = (1.0 - ((r - 0.55).abs() * 8.0).min(1.0)) * strength;
                let shade = 1.0 - 0.65 * (1.0 - r) * strength;
                let blend = |shift: u32, tint: f32| -> u32 {
                    let base = ((color >> shift) & 255) as f32 * shade;
                    (base * (1.0 - ring) + tint * ring).clamp(0.0, 255.0) as u32
                };
                pixels[y as usize * width + x as usize] =
                    (blend(16, accent[0]) << 16) | (blend(8, accent[1]) << 8) | blend(0, accent[2]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_is_bounded_and_restores_exact_pixels_at_edges() {
        let size = (960, 540);
        let mut pixels = (0..size.0 * size.1).map(|i| i as u32).collect::<Vec<_>>();
        let before = pixels.clone();
        let mut patch = Patch::default();
        for at in [(0, 0), (959, 539), (959, 0), (0, 539), (480, 270)] {
            patch.draw(&mut pixels, size, at, 0.5, true, Mode::Full);
            assert_ne!(pixels, before);
            for (i, (&a, &b)) in pixels.iter().zip(&before).enumerate() {
                if a != b {
                    assert!((i % size.0).abs_diff(at.0) <= RADIUS as usize);
                    assert!((i / size.0).abs_diff(at.1) <= RADIUS as usize);
                }
            }
            assert!(patch.saved.len() <= 89 * 89);
            assert!(patch.restore(&mut pixels, size));
            assert_eq!(pixels, before);
            assert!(!patch.restore(&mut pixels, size));
        }
        patch.draw(&mut pixels, size, (960, 0), 0.5, true, Mode::Full);
        assert_eq!(pixels, before);
    }

    #[test]
    fn modes_fade_and_clear_without_residue() {
        let size = (128, 128);
        let before = vec![0x224466; size.0 * size.1];
        let mut pixels = before.clone();
        let mut patch = Patch::default();
        assert_eq!(Mode::Full.next().label(), "REDUCED");
        assert_eq!(Mode::Reduced.next().label(), "OFF");
        assert_eq!(Mode::Off.next().label(), "FULL");
        patch.draw(&mut pixels, size, (64, 64), 0.5, true, Mode::Full);
        let full = pixels.iter().zip(&before).filter(|(a, b)| a != b).count();
        patch.draw(&mut pixels, size, (64, 64), 0.5, false, Mode::Reduced);
        let reduced = pixels.iter().zip(&before).filter(|(a, b)| a != b).count();
        assert!(reduced > 0 && reduced < full);
        assert!(patch.saved.len() <= 53 * 53);
        patch.draw(&mut pixels, size, (64, 64), 0.5, false, Mode::Off);
        assert_eq!(pixels, before);
        for progress in [0.0, 1.0, f32::NAN] {
            patch.draw(&mut pixels, size, (64, 64), progress, true, Mode::Full);
            assert_eq!(pixels, before);
        }
        patch.draw(&mut pixels, size, (64, 64), 0.5, true, Mode::Full);
        patch.draw(&mut pixels, size, (5, 5), 0.5, true, Mode::Full);
        assert_eq!(pixels[64 * size.0 + 64], before[64 * size.0 + 64]);
        assert!(patch.restore(&mut pixels, size));
        assert_eq!(pixels, before);
    }
}
