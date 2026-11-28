//! DPI-aware logical topology built from explicit oriented pixel dimensions.
//! No platform hooks, device identity, or authority is inferred here.
use crate::{
    input::MAX_SLOTS,
    screen_topology::{Edge, Screen, Topology},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    /// Current oriented pixel dimensions, not the unrotated panel dimensions.
    pub width: u16,
    pub height: u16,
    /// Effective OS scale in whole percent, not physical PPI.
    pub scale: u16,
    /// Clockwise orientation of the already-oriented pixel dimensions.
    pub rotation: u16,
}

impl Display {
    pub fn logical(self) -> Option<(u16, u16)> {
        if !(2..=16384).contains(&self.width)
            || !(2..=16384).contains(&self.height)
            || !(50..=400).contains(&self.scale)
            || !matches!(self.rotation, 0 | 90 | 180 | 270)
        {
            return None;
        }
        let scale = u32::from(self.scale);
        let extent = |n: u16| {
            let value = (u32::from(n) * 100 + scale / 2) / scale;
            (2..=16384).contains(&value).then_some(value as u16)
        };
        Some((extent(self.width)?, extent(self.height)?))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub id: u8,
    pub x: i32,
    pub y: i32,
    pub display: Display,
}

impl Placement {
    pub fn screen(self) -> Option<Screen> {
        let (width, height) = self.display.logical()?;
        let screen = Screen {
            id: self.id,
            x: self.x,
            y: self.y,
            width,
            height,
        };
        screen.valid().then_some(screen)
    }
}

/// Exact, contiguous slots, with BASE anchored at the logical origin.
pub fn topology(placements: &[Placement]) -> Option<Topology> {
    if !(2..=MAX_SLOTS + 1).contains(&placements.len())
        || placements[0].id != 0
        || placements[0].x != 0
        || placements[0].y != 0
        || placements
            .iter()
            .enumerate()
            .any(|(i, p)| usize::from(p.id) != i)
    {
        return None;
    }
    Topology::new(
        &placements
            .iter()
            .map(|p| p.screen())
            .collect::<Option<Vec<_>>>()?,
    )
}

/// Snap a dragged CLIENT to the closest touching edge of another screen.
/// An offset along that edge is preserved. Never overlap or move BASE.
pub fn snapped(
    placements: &[Placement],
    id: u8,
    desired: (i32, i32),
    tolerance: u32,
) -> Option<Placement> {
    let index = usize::from(id);
    if index == 0 || index >= placements.len() || tolerance > 16384 {
        return None;
    }
    let mut candidate = placements[index];
    candidate.x = desired.0;
    candidate.y = desired.1;
    let screen = candidate.screen()?;
    let mut trial = placements.to_vec();
    trial[index] = candidate;
    let mut best: Option<(u32, Placement)> = None;
    for anchor in placements.iter().copied().filter(|p| p.id != id) {
        let other = anchor.screen()?;
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            let mut next = candidate;
            let gap = match edge {
                Edge::Left if screen.y < other.bottom() && other.y < screen.bottom() => {
                    next.x = other.x.checked_sub(i32::from(screen.width))?;
                    next.x.abs_diff(desired.0)
                }
                Edge::Right if screen.y < other.bottom() && other.y < screen.bottom() => {
                    next.x = other.right();
                    next.x.abs_diff(desired.0)
                }
                Edge::Top if screen.x < other.right() && other.x < screen.right() => {
                    next.y = other.y.checked_sub(i32::from(screen.height))?;
                    next.y.abs_diff(desired.1)
                }
                Edge::Bottom if screen.x < other.right() && other.x < screen.right() => {
                    next.y = other.bottom();
                    next.y.abs_diff(desired.1)
                }
                _ => continue,
            };
            if gap > tolerance || best.as_ref().is_some_and(|(distance, _)| gap >= *distance) {
                continue;
            }
            trial[index] = next;
            if topology(&trial).is_some() {
                best = Some((gap, next));
            }
        }
    }
    if let Some((_, best)) = best {
        return Some(best);
    }
    trial[index] = candidate;
    topology(&trial).map(|_| candidate)
}

/// Project logical rectangles into a bounded pixel region. Both the hit test
/// and rectangle edges use the same exact integer ratio and half-open bounds.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    origin: (i32, i32),
    offset: (i32, i32),
    extent: (i32, i32),
    numerator: i64,
    denominator: i64,
}

impl Viewport {
    pub fn fit(topology: &Topology, width: u16, height: u16) -> Option<Self> {
        if !(2..=16384).contains(&width) || !(2..=16384).contains(&height) {
            return None;
        }
        let screens = topology.screens();
        let left = screens.iter().map(|s| s.x).min()?;
        let top = screens.iter().map(|s| s.y).min()?;
        let w = i64::from(screens.iter().map(|s| s.right()).max()? - left);
        let h = i64::from(screens.iter().map(|s| s.bottom()).max()? - top);
        let (numerator, denominator) = if i64::from(width) * h <= i64::from(height) * w {
            (i64::from(width), w)
        } else {
            (i64::from(height), h)
        };
        let ceil = |extent: i64| ((extent * numerator + denominator - 1) / denominator) as i32;
        let extent = (ceil(w), ceil(h));
        let result = Self {
            origin: (left, top),
            offset: (
                (i32::from(width) - extent.0) / 2,
                (i32::from(height) - extent.1) / 2,
            ),
            extent,
            numerator,
            denominator,
        };
        // A screen which rounds to fewer than two pixels is not operable.
        if screens.iter().any(|s| {
            let (l, t, r, b) = result.rect(*s);
            r - l < 2 || b - t < 2
        }) {
            return None;
        }
        Some(result)
    }

    pub fn rect(self, screen: Screen) -> (i32, i32, i32, i32) {
        let x = |value: i32| {
            self.offset.0
                + ((i64::from(value - self.origin.0) * self.numerator + self.denominator - 1)
                    / self.denominator) as i32
        };
        let y = |value: i32| {
            self.offset.1
                + ((i64::from(value - self.origin.1) * self.numerator + self.denominator - 1)
                    / self.denominator) as i32
        };
        (
            x(screen.x),
            y(screen.y),
            x(screen.right()),
            y(screen.bottom()),
        )
    }

    pub fn point(self, x: i32, y: i32) -> Option<(i32, i32)> {
        let (x, y) = (x - self.offset.0, y - self.offset.1);
        if !(0..self.extent.0).contains(&x) || !(0..self.extent.1).contains(&y) {
            return None;
        }
        Some((
            self.origin.0 + (i64::from(x) * self.denominator / self.numerator) as i32,
            self.origin.1 + (i64::from(y) * self.denominator / self.numerator) as i32,
        ))
    }

    /// Convert a bounded canvas drag delta to logical units (rounded).
    pub fn delta(self, x: i32, y: i32) -> (i32, i32) {
        let map = |value: i32| {
            let n = i64::from(value) * self.denominator;
            ((n + n.signum() * self.numerator / 2) / self.numerator).clamp(-131072, 131072) as i32
        };
        (map(x), map(y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: u8, x: i32, y: i32, w: u16, h: u16, scale: u16, rotation: u16) -> Placement {
        Placement {
            id,
            x,
            y,
            display: Display {
                width: w,
                height: h,
                scale,
                rotation,
            },
        }
    }

    #[test]
    fn oriented_pixels_and_fractional_scale_are_not_rotated_twice() {
        assert_eq!(
            p(0, 0, 0, 3840, 2160, 150, 0).display.logical(),
            Some((2560, 1440))
        );
        assert_eq!(
            p(1, 0, 0, 1080, 1920, 125, 90).display.logical(),
            Some((864, 1536))
        );
        assert_eq!(p(1, 0, 0, 1080, 1920, 0, 90).display.logical(), None);
        assert_eq!(p(1, 0, 0, 1080, 1920, 100, 45).display.logical(), None);
        assert_eq!(p(1, 0, 0, 16384, 16384, 50, 0).display.logical(), None);
        assert!(
            topology(&[
                p(0, 1, 0, 1920, 1080, 100, 0),
                p(1, 1921, 0, 1080, 1920, 100, 90)
            ])
            .is_none()
        );
    }

    #[test]
    fn projection_agrees_with_every_pixel_of_mixed_topology() {
        let placements = [
            p(0, 0, 0, 3840, 2160, 150, 0),
            p(1, -864, 200, 1080, 1920, 125, 90),
            p(2, 2560, -100, 1600, 900, 100, 0),
        ];
        let t = topology(&placements).unwrap();
        let v = Viewport::fit(&t, 601, 377).unwrap();
        for y in 0..377 {
            for x in 0..601 {
                let from_point = v.point(x, y).and_then(|(x, y)| t.at(x, y));
                let from_rect = t
                    .screens()
                    .iter()
                    .find(|s| {
                        let (l, t, r, b) = v.rect(**s);
                        (l..r).contains(&x) && (t..b).contains(&y)
                    })
                    .map(|s| s.id);
                assert_eq!(from_point, from_rect);
            }
        }
        assert!(t.connected());
        assert_eq!(t.cross(0, Edge::Left, 200).unwrap().target, 1);
        assert_eq!(t.cross(0, Edge::Left, 199), None);
    }

    #[test]
    fn snapping_preserves_offsets_and_does_not_overwrite_other_cards() {
        let mut placements = [
            p(0, 0, 0, 1920, 1080, 100, 0),
            p(1, 1920, 0, 1080, 1920, 100, 90),
            p(2, 3000, 0, 1280, 720, 100, 0),
        ];
        let next = snapped(&placements, 1, (-1091, 170), 32).unwrap();
        assert_eq!((next.x, next.y), (-1080, 170));
        placements[1] = next;
        assert!(!topology(&placements).unwrap().connected());
        assert!(snapped(&placements, 1, (500, 0), 8).is_none());
        assert!(snapped(&placements, 0, (100, 0), 64).is_none());
        placements[2] = snapped(&placements, 2, (1930, 0), 32).unwrap();
        assert_eq!(placements[2].x, 1920);
        assert!(topology(&placements).unwrap().connected());
    }
}
