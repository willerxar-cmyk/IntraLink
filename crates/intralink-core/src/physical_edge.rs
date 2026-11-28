//! Native-edge geometry and a bounded, read-only intent detector.
//! A proposal never grants input or activates a session.
use crate::{
    display_geometry::Display,
    screen_topology::{Edge, Topology, Transfer},
};
use std::time::Duration;

const DWELL: Duration = Duration::from_millis(80);
const MAX_SAMPLE_GAP: Duration = Duration::from_millis(100);
const MAX_TANGENTIAL_DRIFT: u16 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalScreen {
    x: i32,
    y: i32,
    width: u16,
    height: u16,
}
impl PhysicalScreen {
    pub fn new(x: i32, y: i32, width: u16, height: u16) -> Option<Self> {
        if !(2..=16384).contains(&width) || !(2..=16384).contains(&height) {
            return None;
        }
        x.checked_add(i32::from(width))?;
        y.checked_add(i32::from(height))?;
        Some(Self {
            x,
            y,
            width,
            height,
        })
    }
    pub fn size(self) -> (u16, u16) {
        (self.width, self.height)
    }
    pub fn origin(self) -> (i32, i32) {
        (self.x, self.y)
    }
    pub fn offset(self, point: (i32, i32)) -> Option<(u16, u16)> {
        let x = point.0.checked_sub(self.x)?;
        let y = point.1.checked_sub(self.y)?;
        if !(0..i32::from(self.width)).contains(&x) || !(0..i32::from(self.height)).contains(&y) {
            return None;
        }
        Some((x as u16, y as u16))
    }
    pub fn logical_point(self, display: Display, point: (i32, i32)) -> Option<(u16, u16)> {
        if self.size() != (display.width, display.height) {
            return None;
        }
        let (w, h) = display.logical()?;
        let (x, y) = self.offset(point)?;
        Some((scale(x, self.width, w), scale(y, self.height, h)))
    }
    pub fn from_normalized(self, x: u16, y: u16) -> (i32, i32) {
        let map = |value: u16, extent: u16| {
            ((u64::from(value) * u64::from(extent - 1) + 32767) / 65535) as i32
        };
        (self.x + map(x, self.width), self.y + map(y, self.height))
    }
    /// Only an exact single outer edge. Corners and off-monitor samples
    /// are deliberately ambiguous; a corner can never choose a remote.
    fn edge(self, point: (i32, i32)) -> Option<(Edge, u16)> {
        let (x, y) = self.offset(point)?;
        let horizontal = if x == 0 {
            Some(Edge::Left)
        } else if x == self.width - 1 {
            Some(Edge::Right)
        } else {
            None
        };
        let vertical = if y == 0 {
            Some(Edge::Top)
        } else if y == self.height - 1 {
            Some(Edge::Bottom)
        } else {
            None
        };
        match (horizontal, vertical) {
            (Some(e), None) => Some((e, y)),
            (None, Some(e)) => Some((e, x)),
            _ => None,
        }
    }
    fn interior(self, point: (i32, i32)) -> bool {
        self.offset(point)
            .is_some_and(|(x, y)| x > 0 && y > 0 && x < self.width - 1 && y < self.height - 1)
    }
}
fn scale(value: u16, source: u16, target: u16) -> u16 {
    let denominator = u64::from(source - 1);
    ((u64::from(value) * u64::from(target - 1) + denominator / 2) / denominator) as u16
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub edge: Edge,
    pub transfer: Transfer,
}
#[derive(Clone, Copy)]
struct Pending {
    edge: Edge,
    target: u8,
    tangent: u16,
    since: Duration,
}
#[derive(Default)]
pub struct EdgeDetector {
    previous: Option<(i32, i32)>,
    sampled: Option<Duration>,
    pending: Option<Pending>,
}
impl EdgeDetector {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn sample(
        &mut self,
        screen: PhysicalScreen,
        display: Display,
        topology: &Topology,
        point: (i32, i32),
        now: Duration,
    ) -> Option<Proposal> {
        let prior = self.sampled.replace(now);
        let previous = self.previous.replace(point);
        if prior.is_none_or(|last| now < last || now - last > MAX_SAMPLE_GAP) {
            self.pending = None;
            return None;
        }
        let candidate = (|| {
            let base = topology.screens().iter().find(|s| s.id == 0)?;
            let logical = display.logical()?;
            if (base.width, base.height) != logical || base.x != 0 || base.y != 0 {
                return None;
            }
            let (edge, tangent) = screen.edge(point)?;
            let (x, y) = screen.logical_point(display, point)?;
            let transfer = topology.cross(
                0,
                edge,
                if matches!(edge, Edge::Left | Edge::Right) {
                    y
                } else {
                    x
                },
            )?;
            (transfer.target != 0).then_some((edge, tangent, transfer))
        })();
        let Some((edge, tangent, transfer)) = candidate else {
            self.pending = None;
            return None;
        };
        match self.pending {
            Some(p)
                if p.edge == edge
                    && p.target == transfer.target
                    && p.tangent.abs_diff(tangent) <= MAX_TANGENTIAL_DRIFT =>
            {
                if now - p.since >= DWELL {
                    self.pending = None;
                    return Some(Proposal { edge, transfer });
                }
            }
            _ => {
                self.pending = None;
                // A stationary edge at startup, a corner, or sliding
                // from another edge cannot establish new intent.
                if previous.is_some_and(|p| screen.interior(p)) {
                    self.pending = Some(Pending {
                        edge,
                        target: transfer.target,
                        tangent,
                        since: now,
                    });
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_topology::Screen;
    fn fixture() -> (PhysicalScreen, Display, Topology) {
        let display = Display {
            width: 3840,
            height: 2160,
            scale: 150,
            rotation: 0,
        };
        let physical = PhysicalScreen::new(-3840, 100, 3840, 2160).unwrap();
        let topology = Topology::new(&[
            Screen {
                id: 0,
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
            },
            Screen {
                id: 1,
                x: 2560,
                y: 200,
                width: 864,
                height: 1000,
            },
            Screen {
                id: 2,
                x: -500,
                y: 0,
                width: 500,
                height: 720,
            },
        ])
        .unwrap();
        (physical, display, topology)
    }
    #[test]
    fn physical_projection_preserves_endpoints_and_rejects_wrong_dimensions() {
        let (p, d, _) = fixture();
        assert_eq!(p.logical_point(d, (-3840, 100)), Some((0, 0)));
        assert_eq!(p.logical_point(d, (-1, 2259)), Some((2559, 1439)));
        assert_eq!(p.logical_point(d, (0, 2259)), None);
        assert_eq!(
            p.logical_point(Display { width: 1920, ..d }, (-1, 100)),
            None
        );
        assert_eq!(p.from_normalized(0, 0), (-3840, 100));
        assert_eq!(p.from_normalized(65535, 65535), (-1, 2259));
        assert!(PhysicalScreen::new(i32::MAX, 0, 100, 100).is_none());
    }
    #[test]
    fn dwell_requires_interior_approach_and_fires_once() {
        let (p, d, t) = fixture();
        let mut detector = EdgeDetector::default();
        let edge = (-1, 1000);
        let mut sample = |point, ms| detector.sample(p, d, &t, point, Duration::from_millis(ms));
        assert_eq!(sample(edge, 0), None);
        assert_eq!(sample(edge, 90), None);
        assert_eq!(sample((-20, 1000), 100), None);
        assert_eq!(sample(edge, 110), None);
        assert_eq!(sample(edge, 189), None);
        let proposal = sample(edge, 190).unwrap();
        assert_eq!(proposal.edge, Edge::Right);
        assert_eq!(
            proposal.transfer,
            t.cross(0, Edge::Right, p.logical_point(d, edge).unwrap().1)
                .unwrap()
        );
        assert_eq!(sample(edge, 200), None);
        assert_eq!(sample(edge, 280), None);
    }
    #[test]
    fn corners_gaps_drift_and_stalled_sampling_never_propose() {
        let (p, d, t) = fixture();
        for edge in [(-1, 100), (-1, 101), (-3840, 2259), (0, 1000)] {
            let mut detector = EdgeDetector::default();
            for (point, ms) in [((-20, 1000), 0), (edge, 10), (edge, 90)] {
                assert!(
                    detector
                        .sample(p, d, &t, point, Duration::from_millis(ms))
                        .is_none()
                );
            }
        }
        let mut detector = EdgeDetector::default();
        for (point, ms) in [
            ((-20, 1000), 0),
            ((-1, 1000), 10),
            ((-1, 1040), 40),
            ((-1, 1040), 120),
            ((-20, 1000), 130),
            ((-1, 1000), 140),
            ((-1, 1000), 400),
            ((-1, 1000), 480),
        ] {
            assert!(
                detector
                    .sample(p, d, &t, point, Duration::from_millis(ms))
                    .is_none()
            );
        }
    }
}
