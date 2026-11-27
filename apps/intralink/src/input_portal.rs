//! Small, bounded local visual transition for the spatial pad. The effect
//! never influences pointer coordinates, input authority or wire state.
pub(super) const DURATION_MS: u128 = 220;
const RADIUS: isize = 44;

pub(super) fn draw(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    at: (usize, usize),
    progress: f32,
    confirmed: bool,
) {
    if width.checked_mul(height) != Some(pixels.len())
        || at.0 >= width
        || at.1 >= height
        || !progress.is_finite()
    {
        return;
    }
    let (cx, cy) = (at.0 as isize, at.1 as isize);
    let p = progress.clamp(0.0, 1.0);
    let radius = (RADIUS as f32 * (0.55 + 0.45 * (p * std::f32::consts::PI).sin())) as isize;
    // Copy only a small patch: no per-frame full-canvas allocation.
    let mut source = [0u32; ((RADIUS * 2 + 1) * (RADIUS * 2 + 1)) as usize];
    let stride = (RADIUS * 2 + 1) as usize;
    for dy in -RADIUS..=RADIUS {
        for dx in -RADIUS..=RADIUS {
            let x = (cx + dx).clamp(0, width as isize - 1) as usize;
            let y = (cy + dy).clamp(0, height as isize - 1) as usize;
            source[(dy + RADIUS) as usize * stride + (dx + RADIUS) as usize] =
                pixels[y * width + x];
        }
    }
    let strength = if confirmed { 1.0 - p } else { 0.55 };
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
            let sx = ((dx as f32 * c - dy as f32 * s) * pull)
                .round()
                .clamp(-RADIUS as f32, RADIUS as f32) as isize;
            let sy = ((dx as f32 * s + dy as f32 * c) * pull)
                .round()
                .clamp(-RADIUS as f32, RADIUS as f32) as isize;
            let color = source[(sy + RADIUS) as usize * stride + (sx + RADIUS) as usize];
            let ring = (1.0 - ((r - 0.55).abs() * 8.0).min(1.0)) * strength;
            let shade = 0.35 + 0.65 * r;
            let accent = if confirmed {
                [98.0, 224.0, 190.0]
            } else {
                [100.0, 160.0, 235.0]
            };
            let blend = |shift: u32, tint: f32| -> u32 {
                let base = ((color >> shift) & 255) as f32 * shade;
                (base * (1.0 - ring) + tint * ring).clamp(0.0, 255.0) as u32
            };
            pixels[y as usize * width + x as usize] =
                (blend(16, accent[0]) << 16) | (blend(8, accent[1]) << 8) | blend(0, accent[2]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portal_is_bounded_and_does_not_modify_unrelated_pixels() {
        let mut pixels = vec![0x224466; 960 * 540];
        let before = pixels.clone();
        draw(&mut pixels, 960, 540, (0, 0), 0.25, true);
        assert_ne!(pixels, before);
        assert_eq!(pixels[959], before[959]);
        assert_eq!(pixels[100 * 960], before[100 * 960]);
        let after = pixels.clone();
        draw(&mut pixels, 960, 540, (960, 0), 0.5, true);
        assert_eq!(pixels, after);
    }
}
