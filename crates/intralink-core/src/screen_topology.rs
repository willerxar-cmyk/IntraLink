//! Geometry for a future native edge source. No OS hooks or input authority.
//! Physical/logical units must be chosen by the caller consistently (DPI-aware).
use crate::input::MAX_SLOTS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screen {
    /// 0 = local BASE, 1..=8 = immutable remote identities.
    pub id: u8,
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
}

impl Screen {
    pub(crate) fn valid(self) -> bool {
        usize::from(self.id) <= MAX_SLOTS
            && (2..=16384).contains(&self.width)
            && (2..=16384).contains(&self.height)
            && (-65536..=65536).contains(&self.x)
            && (-65536..=65536).contains(&self.y)
    }
    pub fn right(self) -> i32 {
        self.x + i32::from(self.width)
    }
    pub fn bottom(self) -> i32 {
        self.y + i32::from(self.height)
    }
    fn overlaps(self, other: Self) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    pub target: u8,
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone)]
pub struct Topology {
    screens: Vec<Screen>,
}

impl Topology {
    pub fn new(screens: &[Screen]) -> Option<Self> {
        if !(1..=MAX_SLOTS + 1).contains(&screens.len()) || !screens.iter().any(|s| s.id == 0) {
            return None;
        }
        for (i, s) in screens.iter().copied().enumerate() {
            if !s.valid()
                || screens[..i]
                    .iter()
                    .any(|other| other.id == s.id || s.overlaps(*other))
            {
                return None;
            }
        }
        Some(Self {
            screens: screens.to_vec(),
        })
    }

    pub fn screens(&self) -> &[Screen] {
        &self.screens
    }

    pub fn at(&self, x: i32, y: i32) -> Option<u8> {
        self.screens
            .iter()
            .find(|s| (s.x..s.right()).contains(&x) && (s.y..s.bottom()).contains(&y))
            .map(|s| s.id)
    }

    pub fn normalized(&self, id: u8, x: i32, y: i32) -> Option<(u16, u16)> {
        if self.at(x, y) != Some(id) {
            return None;
        }
        let screen = self.screens.iter().find(|s| s.id == id)?;
        let norm =
            |pixel: i32, extent: u16| ((i64::from(pixel) * 65535) / (i64::from(extent) - 1)) as u16;
        Some((
            norm(x - screen.x, screen.width),
            norm(y - screen.y, screen.height),
        ))
    }

    /// Reachability is geometric only. A disconnected placement remains
    /// valid for editing but is not an operable all-device topology.
    pub fn connected(&self) -> bool {
        let mut seen = 1u16;
        for _ in 0..self.screens.len() {
            for a in self.screens.iter().copied() {
                if seen & (1u16 << a.id) == 0 {
                    continue;
                }
                for b in self.screens.iter().copied() {
                    let horizontal = (a.right() == b.x || b.right() == a.x)
                        && a.y < b.bottom()
                        && b.y < a.bottom();
                    let vertical = (a.bottom() == b.y || b.bottom() == a.y)
                        && a.x < b.right()
                        && b.x < a.right();
                    if horizontal || vertical {
                        seen |= 1u16 << b.id;
                    }
                }
            }
        }
        self.screens.iter().all(|s| seen & (1u16 << s.id) != 0)
    }

    /// Trace the first exit of a sampled straight pointer segment. Reject
    /// diagonal corner exits, gaps, jumps over another screen, and exits
    /// outside the shared edge. Rational comparison avoids float ambiguity.
    pub fn trace(&self, source: u8, from: (i32, i32), to: (i32, i32)) -> Option<Transfer> {
        if self.at(from.0, from.1) != Some(source) {
            return None;
        }
        let destination = self.at(to.0, to.1)?;
        if destination == source {
            return None;
        }
        let s = *self.screens.iter().find(|s| s.id == source)?;
        let dx = i64::from(to.0) - i64::from(from.0);
        let dy = i64::from(to.1) - i64::from(from.1);
        let x = if to.0 < s.x {
            Some((Edge::Left, i64::from(from.0 - s.x), -dx))
        } else if to.0 >= s.right() {
            Some((Edge::Right, i64::from(s.right() - from.0), dx))
        } else {
            None
        };
        let y = if to.1 < s.y {
            Some((Edge::Top, i64::from(from.1 - s.y), -dy))
        } else if to.1 >= s.bottom() {
            Some((Edge::Bottom, i64::from(s.bottom() - from.1), dy))
        } else {
            None
        };
        let (edge, n, d) = match (x, y) {
            (Some(a), Some(b)) => match (a.1 * b.2).cmp(&(b.1 * a.2)) {
                std::cmp::Ordering::Less => a,
                std::cmp::Ordering::Greater => b,
                std::cmp::Ordering::Equal => return None,
            },
            (Some(a), None) | (None, Some(a)) => a,
            _ => return None,
        };
        if d <= 0 || n < 0 || n > d {
            return None;
        }
        let (coordinate, delta, extent) = if matches!(edge, Edge::Left | Edge::Right) {
            (from.1 - s.y, dy, s.height)
        } else {
            (from.0 - s.x, dx, s.width)
        };
        let tangent = i64::from(coordinate) * d + delta * n;
        // A segment running exactly along the source's outer corner is not
        // an unambiguous cross. Direct cross() retains its half-open contract.
        if tangent <= 0 || tangent >= i64::from(extent) * d {
            return None;
        }
        self.cross(source, edge, (tangent / d) as u16)
            .filter(|t| t.target == destination)
    }

    /// Cross one explicitly shared edge at the source's local tangential
    /// coordinate. Gaps, corner-only contact and unrelated screens never
    /// select a peer. Inset one pixel at arrival to avoid immediate bounce.
    /// Native input still requires neutral state, owner approval and ACKs.
    pub fn cross(&self, source: u8, edge: Edge, tangent: u16) -> Option<Transfer> {
        let s = *self.screens.iter().find(|s| s.id == source)?;
        let horizontal = matches!(edge, Edge::Left | Edge::Right);
        if tangent >= if horizontal { s.height } else { s.width } {
            return None;
        }
        let position = if horizontal { s.y } else { s.x } + i32::from(tangent);
        for t in self.screens.iter().copied().filter(|t| t.id != source) {
            let shared = match edge {
                Edge::Left => t.right() == s.x,
                Edge::Right => t.x == s.right(),
                Edge::Top => t.bottom() == s.y,
                Edge::Bottom => t.y == s.bottom(),
            };
            let (start, end) = if horizontal {
                (t.y, t.bottom())
            } else {
                (t.x, t.right())
            };
            if !shared || !(start..end).contains(&position) {
                continue;
            }
            let tangent = position - start;
            let (x, y) = match edge {
                Edge::Left => (i32::from(t.width) - 2, tangent),
                Edge::Right => (1, tangent),
                Edge::Top => (tangent, i32::from(t.height) - 2),
                Edge::Bottom => (tangent, 1),
            };
            let norm =
                |pixel: i32, extent: u16| ((pixel as u64 * 65535) / (u64::from(extent) - 1)) as u16;
            return Some(Transfer {
                target: t.id,
                x: norm(x, t.width),
                y: norm(y, t.height),
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn screen(id: u8, x: i32, y: i32, width: u16, height: u16) -> Screen {
        Screen {
            id,
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn exact_overlap_preserves_tangent_on_mixed_sizes_and_negative_origins() {
        let topology = Topology::new(&[
            screen(0, 0, 0, 1920, 1080),
            screen(1, -1280, 180, 1280, 720),
            screen(2, 1920, -200, 1080, 1920),
        ])
        .unwrap();
        assert_eq!(topology.cross(0, Edge::Left, 179), None);
        assert_eq!(topology.cross(0, Edge::Left, 900), None);
        assert_eq!(
            topology.cross(0, Edge::Left, 180),
            Some(Transfer {
                target: 1,
                x: 65483,
                y: 0
            })
        );
        assert_eq!(
            topology.cross(0, Edge::Right, 0),
            Some(Transfer {
                target: 2,
                x: 60,
                y: 6830
            })
        );
        assert_eq!(
            topology.cross(1, Edge::Right, 0),
            Some(Transfer {
                target: 0,
                x: 34,
                y: 10932
            })
        );
    }
    #[test]
    fn vertically_split_neighbors_have_unique_half_open_boundary() {
        let topology = Topology::new(&[
            screen(0, 0, 0, 1000, 1000),
            screen(1, 1000, 0, 500, 500),
            screen(2, 1000, 500, 500, 500),
        ])
        .unwrap();
        assert_eq!(topology.cross(0, Edge::Right, 499).unwrap().target, 1);
        assert_eq!(topology.cross(0, Edge::Right, 500).unwrap().target, 2);
        assert_eq!(topology.cross(0, Edge::Right, 1000), None);
    }
    #[test]
    fn sampled_segments_must_cross_the_actual_shared_edge() {
        let t = Topology::new(&[
            screen(0, 0, 0, 1000, 1000),
            screen(1, 1000, 200, 500, 500),
            screen(2, 1500, 200, 500, 500),
            screen(3, -500, 0, 500, 500),
        ])
        .unwrap();
        assert!(t.connected());
        assert_eq!(
            t.trace(0, (900, 300), (1100, 500)).unwrap(),
            t.cross(0, Edge::Right, 400).unwrap()
        );
        assert_eq!(t.trace(0, (900, 0), (1100, 210)), None); // exits into gap
        assert_eq!(t.trace(0, (900, 300), (1600, 300)), None); // skips screen 1
        assert_eq!(t.trace(1, (1100, 300), (900, 300)).unwrap().target, 0);
        assert_eq!(t.trace(0, (0, 0), (-100, 0)), None); // along corner
        assert_eq!(t.trace(0, (0, 300), (-100, 300)).unwrap().target, 3);
        assert_eq!(t.normalized(0, 999, 999), Some((65535, 65535)));
        assert_eq!(t.normalized(0, 1000, 999), None);
        let isolated =
            Topology::new(&[screen(0, 0, 0, 100, 100), screen(1, 101, 0, 100, 100)]).unwrap();
        assert!(!isolated.connected());
    }

    #[test]
    fn gap_corner_overlap_and_invalid_geometry_fail_closed() {
        let local = screen(0, 0, 0, 1000, 1000);
        let topology = Topology::new(&[
            local,
            screen(1, 1000, 1000, 500, 500),
            screen(2, -501, 0, 500, 500),
        ])
        .unwrap();
        assert_eq!(topology.cross(0, Edge::Right, 999), None);
        assert_eq!(topology.cross(0, Edge::Left, 0), None);
        assert!(Topology::new(&[local, screen(1, 999, 0, 500, 500)]).is_none());
        assert!(Topology::new(&[local, screen(0, 1000, 0, 500, 500)]).is_none());
        assert!(Topology::new(&[local, screen(1, 1000, 0, 1, 500)]).is_none());
        assert!(Topology::new(&[screen(1, 0, 0, 1000, 1000)]).is_none());
    }
}
