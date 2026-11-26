//! Local receiver policy from concurrent software decoder measurements.
//! No GPU/vendor assumptions or unmeasured multicore scaling factors.
use crate::{profile_limits::ReceiverLimits, workspace::MAX_DEVICES};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct DecodeTier {
    pub height: u16,
    /// Worst observed frame or batch period while every slot was decoding.
    pub period: Duration,
}

#[derive(Debug)]
pub struct DecodeCohort {
    pub height: u16,
    /// In selected-device order; one independently measured worker per slot.
    pub slots: Vec<DecodeTier>,
}

/// Choose one measured simultaneous layout, never mix independently measured
/// cohorts. Per-slot manual caps remain absolute. Reserve 25% of elapsed
/// time for presentation, network and other work, not for additional workers
/// (the simultaneous worker load was already present in the measurement).
pub fn recommend(
    cohorts: &[DecodeCohort],
    ceilings: &[ReceiverLimits],
) -> Option<Vec<ReceiverLimits>> {
    if !(1..=MAX_DEVICES).contains(&ceilings.len()) || !(1..=3).contains(&cohorts.len()) {
        return None;
    }
    let maximum = ceilings.iter().map(|limit| limit.height()).max()?;
    let mut seen = 0u8;
    let mut preferred: Option<(u16, Vec<ReceiverLimits>)> = None;
    // Lowest fraction of each slot's requested target, then total FPS,
    // then resolution. This avoids sacrificing a slow slot for fast ones.
    let mut fastest: Option<((u32, u32, u16), Vec<ReceiverLimits>)> = None;
    for cohort in cohorts {
        let bit = match cohort.height {
            540 => 1,
            720 => 2,
            1080 => 4,
            _ => return None,
        };
        if seen & bit != 0 || cohort.height > maximum || cohort.slots.len() != ceilings.len() {
            return None;
        }
        seen |= bit;
        let mut selected = Vec::with_capacity(ceilings.len());
        let mut minimum_ratio = 1000;
        let mut total_fps = 0;
        for (tier, ceiling) in cohort.slots.iter().zip(ceilings) {
            if tier.height != cohort.height.min(ceiling.height()) || tier.period.is_zero() {
                return None;
            }
            let fps =
                (750_000_000u128 / tier.period.as_nanos()).min(u128::from(ceiling.fps())) as u32;
            let Some(limits) = ReceiverLimits::new(tier.height, fps) else {
                continue;
            };
            let target = ceiling.fps().min(30);
            minimum_ratio = minimum_ratio.min((fps * 1000 / target).min(1000));
            total_fps += fps;
            selected.push(limits);
        }
        // A layout that cannot sustain even one FPS on any slot is infeasible.
        if selected.len() != ceilings.len() {
            continue;
        }
        if minimum_ratio == 1000
            && preferred
                .as_ref()
                .is_none_or(|(height, _)| cohort.height > *height)
        {
            preferred = Some((cohort.height, selected.clone()));
        }
        let score = (minimum_ratio, total_fps, cohort.height);
        if fastest.as_ref().is_none_or(|(best, _)| score > *best) {
            fastest = Some((score, selected));
        }
    }
    preferred
        .map(|(_, limits)| limits)
        .or_else(|| fastest.map(|(_, limits)| limits))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(height: u16, fps: u32) -> ReceiverLimits {
        ReceiverLimits::new(height, fps).unwrap()
    }
    fn cohort(height: u16, timings: &[(u16, u64)]) -> DecodeCohort {
        DecodeCohort {
            height,
            slots: timings
                .iter()
                .map(|&(height, ms)| DecodeTier {
                    height,
                    period: Duration::from_millis(ms),
                })
                .collect(),
        }
    }

    #[test]
    fn eight_measured_workers_do_not_divide_their_budget_again() {
        let caps = vec![ReceiverLimits::MAX; 8];
        let cohorts = [cohort(1080, &[(1080, 10); 8])];
        assert_eq!(recommend(&cohorts, &caps), Some(caps));
    }

    #[test]
    fn mixed_ceilings_use_one_observed_cohort_and_keep_manual_bounds() {
        let caps = [ReceiverLimits::MAX, limits(720, 24), limits(540, 15)];
        let tiers = [
            cohort(1080, &[(1080, 30), (720, 10), (540, 10)]),
            cohort(720, &[(720, 15), (720, 10), (540, 10)]),
            cohort(540, &[(540, 5); 3]),
        ];
        assert_eq!(
            recommend(&tiers, &caps),
            Some(vec![limits(720, 50), caps[1], caps[2]])
        );
        let strong = [cohort(1080, &[(1080, 10), (720, 10), (540, 10)])];
        assert_eq!(recommend(&strong, &caps), Some(caps.to_vec()));
    }

    #[test]
    fn slow_devices_choose_best_balanced_feasible_layout() {
        let caps = [ReceiverLimits::MAX, ReceiverLimits::MAX];
        let tiers = [
            cohort(1080, &[(1080, 200), (1080, 10)]),
            cohort(720, &[(720, 100), (720, 80)]),
            cohort(540, &[(540, 50), (540, 60)]),
        ];
        assert_eq!(
            recommend(&tiers, &caps),
            Some(vec![limits(540, 15), limits(540, 12)])
        );
        assert!(recommend(&[cohort(540, &[(540, 1000), (540, 10)])], &caps).is_none());
    }

    #[test]
    fn malformed_or_unmeasured_layouts_are_rejected() {
        let caps = [ReceiverLimits::MAX, limits(540, 30)];
        assert!(recommend(&[cohort(1080, &[(1080, 10)])], &caps).is_none());
        assert!(recommend(&[cohort(1080, &[(1080, 10), (720, 10)])], &caps).is_none());
        assert!(recommend(&[cohort(540, &[(540, 0), (540, 10)])], &caps).is_none());
        assert!(recommend(&[cohort(1440, &[(1080, 10), (540, 10)])], &caps).is_none());
        assert!(
            recommend(
                &[cohort(540, &[(540, 10); 2]), cohort(540, &[(540, 20); 2])],
                &caps
            )
            .is_none()
        );
        assert!(recommend(&[], &caps).is_none());
        assert!(recommend(&[cohort(540, &[(540, 10); 9])], &[ReceiverLimits::MAX; 9]).is_none());
    }
}
