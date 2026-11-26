//! Conservative local receiver policy from measured software decode work.
//! No GPU/vendor assumptions, network data or remote input are used.
use crate::{profile_limits::ReceiverLimits, workspace::MAX_DEVICES};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct DecodeTier {
    pub height: u16,
    /// Slowest observed steady decode of representative local fixtures.
    pub worst: Duration,
}

pub fn recommend(
    tiers: &[DecodeTier],
    ceiling: ReceiverLimits,
    slots: usize,
) -> Option<ReceiverLimits> {
    if !(1..=MAX_DEVICES).contains(&slots) || tiers.len() > 3 || tiers.is_empty() {
        return None;
    }
    let mut seen = 0u8;
    let mut preferred: Option<ReceiverLimits> = None;
    let mut fastest: Option<ReceiverLimits> = None;
    let target = ceiling.fps().min(30);
    for tier in tiers {
        let bit = match tier.height {
            540 => 1,
            720 => 2,
            1080 => 4,
            _ => return None,
        };
        if seen & bit != 0 || tier.worst.is_zero() {
            return None;
        }
        seen |= bit;
        if tier.height > ceiling.height() {
            continue;
        }
        // Reserve 25% for presentation, network and other work; share the
        // remaining single-worker budget across all selected workspace slots.
        // This is deliberately conservative, not a multicore throughput model.
        let cost = tier.worst.as_nanos().checked_mul(slots as u128)?;
        let fps = (750_000_000u128 / cost).min(u128::from(ceiling.fps())) as u32;
        let Some(limits) = ReceiverLimits::new(tier.height, fps) else {
            continue;
        };
        if fps >= target && preferred.is_none_or(|best| limits.height() > best.height()) {
            preferred = Some(limits);
        }
        if fastest.is_none_or(|best| (limits.fps(), limits.height()) > (best.fps(), best.height()))
        {
            fastest = Some(limits);
        }
    }
    preferred.or(fastest)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tier(height: u16, ms: u64) -> DecodeTier {
        DecodeTier {
            height,
            worst: Duration::from_millis(ms),
        }
    }
    #[test]
    fn prefers_1080_when_it_meets_rate_and_never_exceeds_user_limits() {
        let tiers = [tier(1080, 10), tier(720, 5), tier(540, 2)];
        assert_eq!(
            recommend(&tiers, ReceiverLimits::MAX, 1),
            Some(ReceiverLimits::MAX)
        );
        assert_eq!(
            recommend(&tiers, ReceiverLimits::new(720, 24).unwrap(), 1),
            ReceiverLimits::new(720, 24)
        );
        assert_eq!(
            recommend(&tiers, ReceiverLimits::MAX, 4),
            ReceiverLimits::new(720, 37)
        );
        assert_eq!(
            recommend(&tiers, ReceiverLimits::MAX, 8),
            ReceiverLimits::new(540, 46)
        );
    }
    #[test]
    fn slow_devices_select_fastest_feasible_tier_and_bad_evidence_fails_closed() {
        let tiers = [tier(1080, 100), tier(720, 75), tier(540, 50)];
        assert_eq!(
            recommend(&tiers, ReceiverLimits::MAX, 1),
            ReceiverLimits::new(540, 15)
        );
        assert!(recommend(&[tier(540, 1000)], ReceiverLimits::MAX, 1).is_none());
        assert!(recommend(&[tier(540, 0)], ReceiverLimits::MAX, 1).is_none());
        assert!(recommend(&[tier(540, 1), tier(540, 2)], ReceiverLimits::MAX, 1).is_none());
        assert!(recommend(&[tier(1440, 1)], ReceiverLimits::MAX, 1).is_none());
        assert!(recommend(&tiers, ReceiverLimits::MAX, 0).is_none());
        assert!(recommend(&tiers, ReceiverLimits::MAX, MAX_DEVICES + 1).is_none());
    }
}
