//! Calm numbered spatial pad; cell zero is always local.
use intralink_core::input_layout::{Layout, Zone};
use minifb::{MouseMode, Window};

pub const WIDTH: usize = 960;
pub const HEIGHT: usize = 540;

pub fn pointer_zone(window: &Window, layout: Layout) -> Option<Zone> {
    let (x, y) = window.get_mouse_pos(MouseMode::Discard)?;
    let (width, height) = window.get_size();
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
        return None;
    }
    layout.zone(x as usize, y as usize, width, height)
}

// Seven-segment digits 0-8, in order top, upper-right, lower-right,
// bottom, lower-left, upper-left, center. No native font dependency.
const DIGITS: [u8; 9] = [0x3f, 0x06, 0x5b, 0x4f, 0x66, 0x6d, 0x7d, 0x07, 0x7f];

pub fn render(layout: Layout, buffer: &mut [u32], ready: u8, active: Option<usize>, armed: bool) {
    assert_eq!(buffer.len(), WIDTH * HEIGHT);
    buffer.fill(0x00141b25);
    for (cell, pattern) in DIGITS.iter().copied().enumerate().take(layout.cells()) {
        let (l, t, r, b) = layout
            .bounds(cell, WIDTH, HEIGHT)
            .expect("fixed valid pad size");
        let target = cell.checked_sub(1);
        let color = match target {
            None => 0x00414b5a,
            Some(slot) if active == Some(slot) => 0x00216b50,
            Some(slot) if ready & (1 << slot) == 0 => 0x00312c36,
            Some(_) if armed => 0x002e4d75,
            Some(_) => 0x00283b56,
        };
        for y in t + 4..b - 4 {
            buffer[y * WIDTH + l + 4..y * WIDTH + r - 4].fill(color);
        }
        let x = (l + r) / 2 - 20;
        let y = (t + b) / 2 - 32;
        let segments = [
            (x + 5, y, 30, 5),
            (x + 35, y + 5, 5, 25),
            (x + 35, y + 35, 5, 25),
            (x + 5, y + 60, 30, 5),
            (x, y + 35, 5, 25),
            (x, y + 5, 5, 25),
            (x + 5, y + 30, 30, 5),
        ];
        for (index, (sx, sy, w, h)) in segments.into_iter().enumerate() {
            if pattern & (1 << index) != 0 {
                for row in sy..sy + h {
                    buffer[row * WIDTH + sx..row * WIDTH + sx + w].fill(0x00dae5f0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_cell_renders_and_active_state_is_visually_distinct() {
        let layout = Layout::new(8).unwrap();
        let mut pixels = vec![0; WIDTH * HEIGHT];
        render(layout, &mut pixels, 255, Some(7), true);
        let (l, t, _, _) = layout.bounds(8, WIDTH, HEIGHT).unwrap();
        assert_eq!(pixels[(t + 10) * WIDTH + l + 10], 0x00216b50);
        let (l, t, _, _) = layout.bounds(0, WIDTH, HEIGHT).unwrap();
        assert_eq!(pixels[(t + 10) * WIDTH + l + 10], 0x00414b5a);
    }
}
