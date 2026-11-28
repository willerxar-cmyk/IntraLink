//! Pure interaction gate. An arm gesture never grants input until the
//! independent transport acknowledges Activate and the initial Move.
use super::canvas::{self, InputState};
use intralink_core::{display_geometry::Display, overview::Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TargetRegion {
    pub rect: Rect,
    pub display: Display,
    pub video: Option<(usize, usize)>,
    pub window: (usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Idle,
    Activate((u16, u16)),
    Forward((u16, u16)),
    Release,
}

#[derive(Default)]
pub(super) struct Gate {
    pending: Option<TargetRegion>,
    active: Option<TargetRegion>,
}
impl Gate {
    pub fn state(&self) -> InputState {
        if self.active.is_some() {
            InputState::Active
        } else if self.pending.is_some() {
            InputState::Waiting
        } else {
            InputState::Safe
        }
    }
    pub fn arm(&mut self, target: TargetRegion) -> bool {
        if self.state() != InputState::Safe {
            return false;
        }
        self.pending = Some(target);
        true
    }
    pub fn clear(&mut self) {
        self.pending = None;
        self.active = None;
    }
    pub fn acknowledge(&mut self, target: TargetRegion) -> bool {
        if self.pending != Some(target) || self.active.is_some() {
            return false;
        }
        self.pending = None;
        self.active = Some(target);
        true
    }
    pub fn decide(
        &mut self,
        region: Option<TargetRegion>,
        point: Option<(usize, usize)>,
        focused: bool,
        neutral: bool,
        stop: bool,
    ) -> Decision {
        if let Some(active) = self.active {
            if !focused || stop || region != Some(active) {
                return Decision::Release;
            }
            return point
                .and_then(|p| canvas::normalized(active.rect, p))
                .map_or(Decision::Release, Decision::Forward);
        }
        if let Some(pending) = self.pending {
            if !focused || stop || region != Some(pending) {
                self.clear();
                return Decision::Idle;
            }
            if neutral
                && let Some(position) = point.and_then(|p| canvas::normalized(pending.rect, p))
            {
                return Decision::Activate(position);
            }
        }
        Decision::Idle
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn region() -> TargetRegion {
        TargetRegion {
            rect: Rect {
                x: 200,
                y: 220,
                w: 900,
                h: 500,
            },
            display: Display {
                width: 1920,
                height: 1080,
                scale: 100,
                rotation: 0,
            },
            video: Some((1920, 1080)),
            window: (1280, 720),
        }
    }
    #[test]
    fn arm_is_not_activation_and_toolbar_cannot_forward() {
        let t = region();
        let mut gate = Gate::default();
        assert_eq!(
            gate.decide(Some(t), Some((300, 300)), true, true, false),
            Decision::Idle
        );
        assert!(gate.arm(t));
        assert_eq!(
            gate.decide(Some(t), Some((100, 100)), true, true, false),
            Decision::Idle
        );
        assert_eq!(
            gate.decide(Some(t), Some((200, 220)), true, false, false),
            Decision::Idle
        );
        assert_eq!(
            gate.decide(Some(t), Some((200, 220)), true, true, false),
            Decision::Activate((0, 0))
        );
        assert_eq!(gate.state(), InputState::Waiting);
        assert!(gate.acknowledge(t));
        assert_eq!(
            gate.decide(Some(t), Some((1099, 719)), true, true, false),
            Decision::Forward((65535, 65535))
        );
        assert_eq!(
            gate.decide(Some(t), Some((100, 100)), true, false, false),
            Decision::Release
        );
        gate.clear();
        assert_eq!(gate.state(), InputState::Safe);
    }
    #[test]
    fn source_change_resize_focus_loss_or_stop_requires_fresh_arm() {
        let t = region();
        for (next, focused, stop) in [
            (None, true, false),
            (Some(TargetRegion { video: None, ..t }), true, false),
            (
                Some(TargetRegion {
                    window: (800, 600),
                    ..t
                }),
                true,
                false,
            ),
            (Some(t), false, false),
            (Some(t), true, true),
        ] {
            let mut gate = Gate::default();
            assert!(gate.arm(t));
            assert_eq!(
                gate.decide(next, Some((300, 300)), focused, true, stop),
                Decision::Idle
            );
            assert_eq!(gate.state(), InputState::Safe);
            assert!(gate.arm(t));
            assert!(gate.acknowledge(t));
            assert_eq!(
                gate.decide(next, Some((300, 300)), focused, true, stop),
                Decision::Release
            );
            gate.clear();
            assert_eq!(gate.state(), InputState::Safe);
        }
    }
}
