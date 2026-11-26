//! Portable, bounded JPEG quality adaptation for the existing v8 stream.
//! Receipt backlog is an application-pressure hint, not measured bandwidth.
use crate::desktop::{MAX_IN_FLIGHT_RECORDS, VideoConfig, ViewPriority};

pub const MIN_JPEG_QUALITY: u8 = 35;
const HIGH_SAMPLES: u8 = 8;
const LOW_SAMPLES: u8 = 60;
const DOWN_STEP: u8 = 10;
const UP_STEP: u8 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptPressure {
    Low,
    Neutral,
    High,
}

impl ReceiptPressure {
    /// Observe outstanding records before a possible credit wait. Background
    /// pacing is intentional; do not mistake it for a quality recovery signal.
    pub fn from_backlog(outstanding: usize, priority: ViewPriority) -> Self {
        if priority == ViewPriority::Background {
            Self::Neutral
        } else if outstanding >= MAX_IN_FLIGHT_RECORDS - 2 {
            Self::High
        } else if outstanding <= 2 {
            Self::Low
        } else {
            Self::Neutral
        }
    }
}

/// The locally approved JPEG quality is an immutable upper bound. Only
/// changed media observations advance the policy, never repeats/no-updates.
/// Lossless keyframes and tiles retain their original exact representation.
#[derive(Debug)]
pub struct AdaptiveQuality {
    ceiling: u8,
    current: u8,
    high: u8,
    low: u8,
}

impl AdaptiveQuality {
    pub fn new(config: VideoConfig) -> Self {
        Self {
            ceiling: config.quality(),
            current: config.quality(),
            high: 0,
            low: 0,
        }
    }

    pub fn current(&self) -> u8 {
        self.current
    }

    /// Return a new quality only when a sustained-pressure transition occurs.
    /// A neutral or static sample interrupts both streaks.
    pub fn observe(&mut self, pressure: ReceiptPressure, changed: bool) -> Option<u8> {
        let previous = self.current;
        match (changed, pressure) {
            (true, ReceiptPressure::High) => {
                self.low = 0;
                self.high += 1;
                if self.high == HIGH_SAMPLES {
                    self.current = self.current.saturating_sub(DOWN_STEP).max(MIN_JPEG_QUALITY);
                    self.high = 0;
                }
            }
            (true, ReceiptPressure::Low) => {
                self.high = 0;
                self.low += 1;
                if self.low == LOW_SAMPLES {
                    self.current = self.current.saturating_add(UP_STEP).min(self.ceiling);
                    self.low = 0;
                }
            }
            _ => {
                self.high = 0;
                self.low = 0;
            }
        }
        (self.current != previous).then_some(self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller(ceiling: u8) -> AdaptiveQuality {
        AdaptiveQuality::new(VideoConfig::new(1080, 60, ceiling, 30).unwrap())
    }

    #[test]
    fn pressure_is_bounded_and_background_does_not_signal_recovery() {
        use ReceiptPressure::{High, Low, Neutral};
        for count in 0..=MAX_IN_FLIGHT_RECORDS {
            let expected = match count {
                0..=2 => Low,
                3..=5 => Neutral,
                _ => High,
            };
            assert_eq!(
                ReceiptPressure::from_backlog(count, ViewPriority::Foreground),
                expected
            );
            assert_eq!(
                ReceiptPressure::from_backlog(count, ViewPriority::Background),
                Neutral
            );
        }
    }

    #[test]
    fn sustained_high_pressure_reduces_quality_and_floor_is_enforced() {
        let mut policy = controller(75);
        for _ in 0..7 {
            assert_eq!(policy.observe(ReceiptPressure::High, true), None);
        }
        assert_eq!(policy.observe(ReceiptPressure::High, true), Some(65));
        for _ in 0..200 {
            policy.observe(ReceiptPressure::High, true);
        }
        assert_eq!(policy.current(), 35);
    }

    #[test]
    fn recovery_is_slower_and_never_exceeds_approved_quality() {
        let mut policy = controller(76);
        for _ in 0..8 {
            policy.observe(ReceiptPressure::High, true);
        }
        assert_eq!(policy.current(), 66);
        for _ in 0..59 {
            assert_eq!(policy.observe(ReceiptPressure::Low, true), None);
        }
        assert_eq!(policy.observe(ReceiptPressure::Low, true), Some(71));
        for _ in 0..200 {
            policy.observe(ReceiptPressure::Low, true);
        }
        assert_eq!(policy.current(), 76);
    }

    #[test]
    fn static_and_neutral_interrupt_streaks_and_floor_is_safe() {
        let mut policy = controller(36);
        for _ in 0..100 {
            for _ in 0..7 {
                assert_eq!(policy.observe(ReceiptPressure::High, true), None);
            }
            assert_eq!(policy.observe(ReceiptPressure::High, false), None);
            assert_eq!(policy.observe(ReceiptPressure::Neutral, true), None);
        }
        for _ in 0..8 {
            policy.observe(ReceiptPressure::High, true);
        }
        assert_eq!(policy.current(), 35);
        for _ in 0..60 {
            policy.observe(ReceiptPressure::Low, true);
        }
        assert_eq!(policy.current(), 36);
    }
}
