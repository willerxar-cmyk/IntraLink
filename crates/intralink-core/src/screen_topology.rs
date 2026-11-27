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
    fn valid(self) -> bool {
        usize::from(self.id) <= MAX_SLOTS
            && (2..=16384).contains(&self.width)
            && (2..=16384).contains(&self.height)
            && (-65536..=65536).contains(&self.x)
            && (-65536..=65536).contains(&self.y)
    }
    fn right(self) -> i32 {
        self.x + i32::from(self.width)
    }
    fn bottom(self) -> i32 {
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
