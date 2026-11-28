//! Relative virtual pointer for a native foreground source. No OS API or
//! input authority. Proposals are committed only after the caller's ACK barrier.
use crate::{
    display_geometry::Display,
    screen_topology::{Screen, Topology, Transfer},
};
use std::time::Duration;

const COOLDOWN: Duration = Duration::from_millis(80);
const MAX_DELTA: i32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Move { x: u16, y: u16 },
    Cross(Transfer),
}

pub struct VirtualPointer {
    id: u8,
    point: (i32, i32),
    remainder: (i32, i32),
    since: Duration,
}

fn at_transfer(t: &Topology, transfer: Transfer) -> Option<(i32, i32)> {
    let s = t.screens().iter().find(|s| s.id == transfer.target)?;
    let axis = |value: u16, extent: u16| {
        ((i64::from(value) * i64::from(extent - 1) + 32767) / 65535) as i32
    };
    let point = (
        s.x + axis(transfer.x, s.width),
        s.y + axis(transfer.y, s.height),
    );
    // Exact canonical normalized values produced by Topology::cross only.
    (t.normalized(s.id, point.0, point.1) == Some((transfer.x, transfer.y))).then_some(point)
}

impl VirtualPointer {
    pub fn enter(t: &Topology, transfer: Transfer, now: Duration) -> Option<Self> {
        if transfer.target == 0 {
            return None;
        }
        Some(Self {
            id: transfer.target,
            point: at_transfer(t, transfer)?,
            remainder: (0, 0),
            since: now,
        })
    }

    pub fn id(&self) -> u8 {
        self.id
    }

    /// Process the ordered native samples until the first crossing. The
    /// caller sends the last in-screen position or the ACK-gated proposal,
    /// and discards any remainder of the batch rather than replaying it
    /// against a different owner. Empty batches still supply a position.
    pub fn motion_sequence(
        &mut self,
        t: &Topology,
        base: Display,
        deltas: impl IntoIterator<Item = (i32, i32)>,
        neutral: bool,
        now: Duration,
    ) -> Option<Motion> {
        let mut result = self.motion(t, base, (0, 0), neutral, now)?;
        for delta in deltas {
            result = self.motion(t, base, delta, neutral, now)?;
            if matches!(result, Motion::Cross(_)) {
                break;
            }
        }
        Some(result)
    }

    /// BASE physical pixel deltas are converted to common logical units
    /// with signed fractional carry. The cursor never accumulates a gap,
    /// skips another device, or transfers a held chord.
    pub fn motion(
        &mut self,
        t: &Topology,
        base: Display,
        delta: (i32, i32),
        neutral: bool,
        now: Duration,
    ) -> Option<Motion> {
        if delta.0.unsigned_abs() > MAX_DELTA as u32
            || delta.1.unsigned_abs() > MAX_DELTA as u32
            || now < self.since
        {
            return None;
        }
        let (w, h) = base.logical()?;
        let screen = *t.screens().iter().find(|s| s.id == self.id)?;
        let source = t.screens().iter().find(|s| s.id == 0)?;
        if (source.width, source.height) != (w, h)
            || t.at(self.point.0, self.point.1) != Some(self.id)
        {
            return None;
        }
        let convert = |value: i32, remainder: i32| {
            let n = value * 100 + remainder;
            (n / i32::from(base.scale), n % i32::from(base.scale))
        };
        let (dx, rx) = convert(delta.0, self.remainder.0);
        let (dy, ry) = convert(delta.1, self.remainder.1);
        let destination = (self.point.0.checked_add(dx)?, self.point.1.checked_add(dy)?);
        if neutral
            && now - self.since >= COOLDOWN
            && let Some(transfer) = t.trace(self.id, self.point, destination)
        {
            // Never mutate owner or cursor on a proposed crossing.
            return Some(Motion::Cross(transfer));
        }
        let next = clamp(screen, destination);
        self.remainder = (
            if next.0 == destination.0 { rx } else { 0 },
            if next.1 == destination.1 { ry } else { 0 },
        );
        self.point = next;
        let (x, y) = t.normalized(self.id, next.0, next.1)?;
        Some(Motion::Move { x, y })
    }
}

fn clamp(s: Screen, p: (i32, i32)) -> (i32, i32) {
    (
        p.0.clamp(s.x, s.right() - 1),
        p.1.clamp(s.y, s.bottom() - 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_topology::Edge;
    fn fixture() -> (Topology, Display) {
        let base = Display {
            width: 1500,
            height: 1200,
            scale: 150,
            rotation: 0,
        };
        let t = Topology::new(&[
            Screen {
                id: 0,
                x: 0,
                y: 0,
                width: 1000,
                height: 800,
            },
            Screen {
                id: 1,
                x: 1000,
                y: 0,
                width: 500,
                height: 800,
            },
            Screen {
                id: 2,
                x: 1500,
                y: 0,
                width: 500,
                height: 800,
            },
        ])
        .unwrap();
        (t, base)
    }
    #[test]
    fn fractional_motion_and_acknowledged_crossings_preserve_owner() {
        let (t, b) = fixture();
        let at = Duration::from_millis;
        let entry = t.cross(0, Edge::Right, 400).unwrap();
        let mut cursor = VirtualPointer::enter(&t, entry, at(0)).unwrap();
        assert_eq!(cursor.id(), 1);
        let p = cursor.point;
        for _ in 0..3 {
            assert!(matches!(
                cursor.motion(&t, b, (1, 0), true, at(10)),
                Some(Motion::Move { .. })
            ));
        }
        assert_eq!(cursor.point.0, p.0 + 2);
        // Can never jump over a screen, even on a large sample.
        assert!(matches!(
            cursor.motion(&t, b, (1800, 0), true, at(100)),
            Some(Motion::Move { .. })
        ));
        assert_eq!(cursor.point.0, 1499);
        let Some(Motion::Cross(cross)) = cursor.motion(&t, b, (3, 0), true, at(101)) else {
            panic!()
        };
        assert_eq!(cross.target, 2);
        assert_eq!(cursor.id(), 1);
        let mut cursor = VirtualPointer::enter(&t, cross, at(101)).unwrap();
        assert!(matches!(
            cursor.motion(&t, b, (-6, 0), true, at(120)),
            Some(Motion::Move { .. })
        ));
        let Some(Motion::Cross(back)) = cursor.motion(&t, b, (-3, 0), true, at(181)) else {
            panic!()
        };
        assert_eq!(back.target, 1);
        let mut cursor = VirtualPointer::enter(&t, back, at(181)).unwrap();
        assert!(matches!(
            cursor.motion(&t, b, (-748, 0), true, at(270)),
            Some(Motion::Move { .. })
        ));
        let Some(Motion::Cross(local)) = cursor.motion(&t, b, (-6, 0), true, at(280)) else {
            panic!()
        };
        assert_eq!(local.target, 0);
        assert_eq!(cursor.id(), 1);
        assert!(VirtualPointer::enter(&t, local, at(280)).is_none());
    }
    #[test]
    fn raw_batch_preserves_first_edge_even_when_later_motion_reverses() {
        let (t, b) = fixture();
        let entry = t.cross(0, Edge::Right, 400).unwrap();
        let mut cursor = VirtualPointer::enter(&t, entry, Duration::ZERO).unwrap();
        let Some(Motion::Cross(next)) =
            cursor.motion_sequence(&t, b, [(750, 0), (-750, 0)], true, COOLDOWN)
        else {
            panic!("First edge must win");
        };
        assert_eq!(next.target, 2);
        assert_eq!(cursor.id(), 1);
        let mut cursor = VirtualPointer::enter(&t, entry, Duration::ZERO).unwrap();
        assert!(matches!(
            cursor.motion_sequence(&t, b, [(750, 0), (-750, 0)], false, COOLDOWN),
            Some(Motion::Move { .. })
        ));
        assert_eq!(cursor.id(), 1);
    }

    #[test]
    fn held_input_never_transfers_and_invalid_samples_fail_closed() {
        let (t, b) = fixture();
        let mut cursor =
            VirtualPointer::enter(&t, t.cross(0, Edge::Right, 400).unwrap(), Duration::ZERO)
                .unwrap();
        assert!(matches!(
            cursor.motion(&t, b, (-120, 0), false, COOLDOWN),
            Some(Motion::Move { x: 0, .. })
        ));
        assert_eq!(cursor.id(), 1);
        assert!(
            cursor
                .motion(&t, b, (i32::MAX, 0), true, COOLDOWN)
                .is_none()
        );
        assert!(cursor.motion(&t, b, (0, 0), true, Duration::ZERO).is_some());
        assert!(
            cursor
                .motion(&t, Display { scale: 0, ..b }, (0, 0), true, COOLDOWN)
                .is_none()
        );
    }
}
