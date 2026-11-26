//! Local Host profile selection from measured capture/encode candidates.
//! No native handles, network requests or vendor assumptions in this policy.
use crate::desktop::VideoConfig;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturePath {
    Snapshot,
    Wgc,
    WgcGpu,
}

#[derive(Clone, Copy, Debug)]
pub struct HostMeasurement {
    pub path: CapturePath,
    pub height: u16,
    /// Worst observed steady capture interval/work including native waiting.
    pub capture: Duration,
    /// Worst actual software encoding time, including resize and comparison.
    pub encode: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostProfile {
    pub path: CapturePath,
    pub config: VideoConfig,
}

/// Reserve a quarter of the measured local service budget. Prefer the
/// highest tested resolution meeting min(30, requested FPS), otherwise the
/// best feasible FPS. Within the same tier prefer the lowest observed cost.
pub fn recommend(approved: VideoConfig, measurements: &[HostMeasurement]) -> Option<HostProfile> {
    if !(1..=9).contains(&measurements.len()) {
        return None;
    }
    let mut seen = 0u16;
    let mut best: Option<((bool, u32, u32, u128), HostProfile)> = None;
    let target = approved.fps().min(30);
    for sample in measurements {
        let height_index = match sample.height {
            540 => 0,
            720 => 1,
            1080 => 2,
            _ => return None,
        };
        let backend_index = match sample.path {
            CapturePath::Snapshot => 0,
            CapturePath::Wgc => 1,
            CapturePath::WgcGpu => 2,
        };
        let bit = 1u16 << (height_index * 3 + backend_index);
        if seen & bit != 0
            || u32::from(sample.height) > approved.height()
            || sample.capture.is_zero()
            || sample.encode.is_zero()
        {
            return None;
        }
        seen |= bit;
        let cost = sample.capture.checked_add(sample.encode)?.as_nanos();
        let fps = (750_000_000u128 / cost).min(u128::from(approved.fps())) as u32;
        let Some(config) =
            VideoConfig::new(sample.height, fps, approved.quality(), approved.seconds())
                .map(|config| config.with_mode(approved.mode()))
        else {
            continue;
        };
        let meets_target = fps >= target;
        let score = if meets_target {
            (true, config.height(), fps, u128::MAX - cost)
        } else {
            (false, fps, config.height(), u128::MAX - cost)
        };
        let candidate = HostProfile {
            path: sample.path,
            config,
        };
        if best.as_ref().is_none_or(|(previous, _)| score > *previous) {
            best = Some((score, candidate));
        }
    }
    best.map(|(_, profile)| profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::CodecMode;

    fn approved() -> VideoConfig {
        VideoConfig::new(1080, 60, 75, 180)
            .unwrap()
            .with_mode(CodecMode::Compact)
    }
    fn sample(path: CapturePath, height: u16, capture: u64, encode: u64) -> HostMeasurement {
        HostMeasurement {
            path,
            height,
            capture: Duration::from_millis(capture),
            encode: Duration::from_millis(encode),
        }
    }

    #[test]
    fn selects_measured_best_backend_at_highest_usable_resolution() {
        let measured = [
            sample(CapturePath::Snapshot, 1080, 200, 5),
            sample(CapturePath::Wgc, 1080, 10, 12),
            sample(CapturePath::WgcGpu, 1080, 5, 8),
            sample(CapturePath::Snapshot, 720, 2, 2),
        ];
        let profile = recommend(approved(), &measured).unwrap();
        assert_eq!(profile.path, CapturePath::WgcGpu);
        assert_eq!(profile.config.height(), 1080);
        assert_eq!(profile.config.fps(), 57);
        assert_eq!(profile.config.mode(), approved().mode());
        assert_eq!(profile.config.quality(), approved().quality());
        assert_eq!(profile.config.seconds(), approved().seconds());
    }

    #[test]
    fn cpu_only_path_can_select_lower_resolution_and_rate() {
        let caps = [
            sample(CapturePath::Snapshot, 1080, 100, 30),
            sample(CapturePath::Snapshot, 720, 25, 5),
            sample(CapturePath::Snapshot, 540, 15, 5),
        ];
        let profile = recommend(approved(), &caps).unwrap();
        assert_eq!(profile.path, CapturePath::Snapshot);
        assert_eq!(profile.config.height(), 540);
        assert_eq!(profile.config.fps(), 37);
        let ceiling = VideoConfig::new(720, 15, 35, 1).unwrap();
        let profile = recommend(ceiling, &[sample(CapturePath::Snapshot, 720, 2, 2)]).unwrap();
        assert_eq!(profile.config, ceiling);
    }

    #[test]
    fn rejects_duplicate_unknown_zero_and_over_ceiling_samples() {
        let valid = sample(CapturePath::Snapshot, 540, 10, 5);
        assert!(recommend(approved(), &[valid, valid]).is_none());
        assert!(recommend(approved(), &[sample(CapturePath::Snapshot, 1440, 10, 5)]).is_none());
        assert!(recommend(approved(), &[sample(CapturePath::Snapshot, 540, 0, 5)]).is_none());
        assert!(recommend(approved(), &[sample(CapturePath::Snapshot, 540, 10, 0)]).is_none());
        assert!(recommend(approved(), &[sample(CapturePath::Snapshot, 540, 900, 10)]).is_none());
        assert!(
            recommend(
                VideoConfig::new(540, 60, 75, 30).unwrap(),
                &[sample(CapturePath::Snapshot, 720, 10, 5)]
            )
            .is_none()
        );
        assert!(recommend(approved(), &[]).is_none());
    }
}
