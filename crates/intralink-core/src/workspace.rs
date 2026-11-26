//! Local workspace policy, independent of the operating system and network.
use crate::desktop::ViewPriority;

pub const MAX_DEVICES: usize = 8;

/// Focus changes presentation and scheduling, not the authorization of a host.
#[derive(Debug, Clone, Copy)]
pub struct WorkspacePolicy {
    count: usize,
    active: u8,
    selected: usize,
    all_live: bool,
}

impl WorkspacePolicy {
    pub fn new(count: usize, all_live: bool) -> Option<Self> {
        if !(1..=MAX_DEVICES).contains(&count) {
            return None;
        }
        Some(Self {
            count,
            active: (u16::from(1u8).checked_shl(count as u32)? - 1) as u8,
            selected: 0,
            all_live,
        })
    }

    pub const fn selected(&self) -> usize {
        self.selected
    }
    pub const fn all_live(&self) -> bool {
        self.all_live
    }
    pub fn set_all_live(&mut self, value: bool) {
        self.all_live = value;
    }

    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.count {
            return false;
        }
        self.selected = index;
        true
    }

    pub fn is_active(&self, index: usize) -> bool {
        index < self.count && self.active & (1 << index) != 0
    }

    pub fn set_active(&mut self, index: usize, value: bool) -> bool {
        if index >= self.count {
            return false;
        }
        if value {
            self.active |= 1 << index;
        } else {
            self.active &= !(1 << index);
        }
        true
    }

    pub fn priority(&self, index: usize) -> Option<ViewPriority> {
        if !self.is_active(index) {
            return None;
        }
        Some(if self.all_live || index == self.selected {
            ViewPriority::Foreground
        } else {
            ViewPriority::Background
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ViewPriority::{Background, Foreground};

    #[test]
    fn three_live_devices_switch_focus_without_deactivation() {
        let mut policy = WorkspacePolicy::new(3, false).unwrap();
        assert_eq!(
            [policy.priority(0), policy.priority(1), policy.priority(2)],
            [Some(Foreground), Some(Background), Some(Background)]
        );
        assert!(policy.select(2));
        assert_eq!(
            [policy.priority(0), policy.priority(1), policy.priority(2)],
            [Some(Background), Some(Background), Some(Foreground)]
        );
        assert!((0..3).all(|index| policy.is_active(index)));
    }

    #[test]
    fn inactive_is_distinct_from_background_and_all_live() {
        let mut policy = WorkspacePolicy::new(3, true).unwrap();
        assert!(policy.set_active(1, false));
        assert!(policy.select(1));
        assert_eq!(policy.priority(1), None);
        assert_eq!(policy.priority(2), Some(Foreground));
        policy.set_all_live(false);
        assert_eq!(policy.priority(2), Some(Background));
        assert!(policy.set_active(1, true));
        assert_eq!(policy.priority(1), Some(Foreground));
    }

    #[test]
    fn bounds_and_all_eight_slots() {
        assert!(WorkspacePolicy::new(0, false).is_none());
        assert!(WorkspacePolicy::new(9, false).is_none());
        let mut policy = WorkspacePolicy::new(8, true).unwrap();
        assert!(policy.all_live());
        assert!((0..8).all(|index| policy.is_active(index)));
        assert!(!policy.select(8));
        assert!(!policy.set_active(usize::MAX, false));
        assert_eq!(policy.selected(), 0);
        assert_eq!(policy.priority(8), None);
        for index in 0..8 {
            assert!(policy.set_active(index, false));
        }
        assert!((0..8).all(|index| !policy.is_active(index)));
    }
}
