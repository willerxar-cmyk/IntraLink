//! Bounded local workspace overview geometry and thumbnail rendering.
//! No native input forwarding, network operations or external dependencies.

pub const WIDTH: usize = 960;
pub const HEIGHT: usize = 540;
const GAP: usize = 10;
const TOP: usize = 52;
const MAX_SLOTS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub fn contains(self, x: usize, y: usize) -> bool {
        x >= self.x && y >= self.y && x - self.x < self.w && y - self.y < self.h
    }
}

pub const MODE: Rect = Rect {
    x: 550,
    y: 10,
    w: 142,
    h: 32,
};
pub const FULLSCREEN: Rect = Rect {
    x: 702,
    y: 10,
    w: 146,
    h: 32,
};
pub const BACK: Rect = Rect {
    x: 858,
    y: 10,
    w: 92,
    h: 32,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Card {
    pub outer: Rect,
    pub preview: Rect,
    pub toggle: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Focus(usize),
    Toggle(usize),
    Mode,
    Fullscreen,
    Back,
}

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    count: usize,
    columns: usize,
    rows: usize,
}

impl Layout {
    pub fn new(count: usize) -> Option<Self> {
        if !(1..=MAX_SLOTS).contains(&count) {
            return None;
        }
        let columns = match count {
            1 => 1,
            2..=4 => 2,
            _ => 3,
        };
        Some(Self {
            count,
            columns,
            rows: count.div_ceil(columns),
        })
    }

    pub fn card(self, index: usize) -> Option<Card> {
        if index >= self.count {
            return None;
        }
        let w = (WIDTH - GAP * (self.columns + 1)) / self.columns;
        let h = (HEIGHT - TOP - GAP * (self.rows + 1)) / self.rows;
        let x = GAP + index % self.columns * (w + GAP);
        let y = TOP + GAP + index / self.columns * (h + GAP);
        Some(Card {
            outer: Rect { x, y, w, h },
            preview: Rect {
                x: x + 6,
                y: y + 34,
                w: w - 12,
                h: h - 56,
            },
            toggle: Rect {
                x: x + w - 68,
                y: y + 5,
                w: 62,
                h: 24,
            },
        })
    }

    pub fn hit(self, x: usize, y: usize) -> Option<Action> {
        if MODE.contains(x, y) {
            return Some(Action::Mode);
        }
        if FULLSCREEN.contains(x, y) {
            return Some(Action::Fullscreen);
        }
        if BACK.contains(x, y) {
            return Some(Action::Back);
        }
        for index in 0..self.count {
            let card = self.card(index)?;
            if card.toggle.contains(x, y) {
                return Some(Action::Toggle(index));
            }
            if card.outer.contains(x, y) {
                return Some(Action::Focus(index));
            }
        }
        None
    }
}

/// Map a native client-area coordinate through the aspect-ratio-preserving
/// presentation. Black bars and invalid or off-window coordinates are inert.
pub fn map_point(size: (usize, usize), point: (f32, f32)) -> Option<(usize, usize)> {
    let (w, h) = (size.0 as f64, size.1 as f64);
    let (x, y) = (f64::from(point.0), f64::from(point.1));
    if w == 0.0 || h == 0.0 || !x.is_finite() || !y.is_finite() {
        return None;
    }
    let scale = (w / WIDTH as f64).min(h / HEIGHT as f64);
    let px = (x - (w - WIDTH as f64 * scale) / 2.0) / scale;
    let py = (y - (h - HEIGHT as f64 * scale) / 2.0) / scale;
    if !(0.0..WIDTH as f64).contains(&px) || !(0.0..HEIGHT as f64).contains(&py) {
        return None;
    }
    Some((px as usize, py as usize))
}

/// Paint a rectangle only within the fixed overview canvas.
pub fn fill(canvas: &mut [u32], rect: Rect, color: u32) -> bool {
    if canvas.len() != WIDTH * HEIGHT
        || rect.w == 0
        || rect.h == 0
        || rect.x.checked_add(rect.w).is_none_or(|end| end > WIDTH)
        || rect.y.checked_add(rect.h).is_none_or(|end| end > HEIGHT)
    {
        return false;
    }
    for y in rect.y..rect.y + rect.h {
        let start = y * WIDTH + rect.x;
        canvas[start..start + rect.w].fill(color);
    }
    true
}

/// Scale a validated decoded surface into a card without intermediate allocations.
/// The nearest-neighbor thumbnail retains aspect ratio; unused areas are black.
pub fn thumbnail(canvas: &mut [u32], rect: Rect, source: &[u32], size: (usize, usize)) -> bool {
    let (sw, sh) = size;
    if !(1..=1920).contains(&sw) || !(1..=1080).contains(&sh) || source.len() != sw * sh {
        return false;
    }
    if !fill(canvas, rect, 0) {
        return false;
    }
    let (dw, dh) = if rect.w * sh <= rect.h * sw {
        (rect.w, (rect.w * sh / sw).max(1))
    } else {
        ((rect.h * sw / sh).max(1), rect.h)
    };
    let x0 = rect.x + (rect.w - dw) / 2;
    let y0 = rect.y + (rect.h - dh) / 2;
    for y in 0..dh {
        let src_row = y * sh / dh * sw;
        let dst = &mut canvas[(y0 + y) * WIDTH + x0..(y0 + y) * WIDTH + x0 + dw];
        for (x, pixel) in dst.iter_mut().enumerate() {
            *pixel = source[src_row + x * sw / dw];
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_layout_has_nonoverlapping_cards_and_distinct_controls() {
        assert!(Layout::new(0).is_none());
        assert!(Layout::new(9).is_none());
        for count in 1..=8 {
            let layout = Layout::new(count).unwrap();
            for index in 0..count {
                let card = layout.card(index).unwrap();
                assert!(
                    card.outer.x + card.outer.w <= WIDTH && card.outer.y + card.outer.h <= HEIGHT
                );
                assert_eq!(
                    layout.hit(card.preview.x, card.preview.y),
                    Some(Action::Focus(index))
                );
                assert_eq!(
                    layout.hit(card.toggle.x, card.toggle.y),
                    Some(Action::Toggle(index))
                );
                for other in 0..index {
                    let prev = layout.card(other).unwrap().outer;
                    assert!(prev.x + prev.w <= card.outer.x || prev.y + prev.h <= card.outer.y);
                }
            }
            assert!(layout.card(count).is_none());
            assert_eq!(layout.hit(0, 0), None);
            for (rect, action) in [
                (MODE, Action::Mode),
                (FULLSCREEN, Action::Fullscreen),
                (BACK, Action::Back),
            ] {
                assert_eq!(layout.hit(rect.x, rect.y), Some(action));
            }
        }
    }

    #[test]
    fn pointer_mapping_respects_bars_and_nonfinite_input() {
        assert_eq!(map_point((1920, 1080), (100.0, 200.0)), Some((50, 100)));
        assert_eq!(map_point((1000, 1000), (500.0, 500.0)), Some((480, 270)));
        assert_eq!(map_point((1000, 1000), (500.0, 100.0)), None);
        assert_eq!(map_point((2000, 540), (100.0, 200.0)), None);
        assert_eq!(map_point((960, 540), (960.0, 0.0)), None);
        assert_eq!(map_point((0, 540), (0.0, 0.0)), None);
        assert_eq!(map_point((960, 540), (f32::NAN, 0.0)), None);
        assert_eq!(map_point((960, 540), (0.0, -1.0)), None);
    }

    #[test]
    fn thumbnail_preserves_aspect_and_does_not_touch_other_cards() {
        let mut canvas = vec![7; WIDTH * HEIGHT];
        let rect = Rect {
            x: 10,
            y: 10,
            w: 8,
            h: 8,
        };
        assert!(thumbnail(&mut canvas, rect, &[1, 2], (2, 1)));
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let expected = if !rect.contains(x, y) {
                    7
                } else if !(12..16).contains(&y) {
                    0
                } else if x < 14 {
                    1
                } else {
                    2
                };
                assert_eq!(canvas[y * WIDTH + x], expected);
            }
        }
    }

    #[test]
    fn invalid_geometry_and_source_leave_canvas_unchanged() {
        let mut canvas = vec![7; WIDTH * HEIGHT];
        let good = Rect {
            x: 10,
            y: 10,
            w: 8,
            h: 8,
        };
        for rect in [
            Rect {
                x: usize::MAX,
                ..good
            },
            Rect { w: 0, ..good },
            Rect { h: HEIGHT, ..good },
        ] {
            assert!(!thumbnail(&mut canvas, rect, &[1], (1, 1)));
        }
        assert!(!thumbnail(&mut canvas, good, &[1], (2, 1)));
        assert!(!thumbnail(&mut canvas, good, &[1], (0, 1)));
        assert!(!fill(&mut canvas[..1], good, 0));
        assert!(canvas.iter().all(|pixel| *pixel == 7));
    }
}
