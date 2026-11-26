//! Local capture pacing, independent of networking and the system clock.
use std::time::Duration;

use crate::desktop::ViewPriority;

const IDLE_LIMIT: Duration = Duration::from_millis(100);

/// Preserve the negotiated maximum FPS while reducing repeated static captures.
/// A changed record immediately resets the next capture interval.
#[derive(Debug)]
pub struct CapturePacer {
    base: Duration,
    repeats: u8,
}

impl CapturePacer {
    pub fn new(config: crate::desktop::VideoConfig) -> Self {
        Self::from_fps(config.fps())
    }

    pub fn for_h264(profile: crate::h264::Profile) -> Self {
        Self::from_fps(profile.fps())
    }

    fn from_fps(fps: u32) -> Self {
        Self {
            base: Duration::from_secs_f64(1.0 / f64::from(fps)),
            repeats: 0,
        }
    }

    /// Background sessions remain connected, with at most five samples per second.
    /// Neither foreground nor background can exceed the host-approved FPS.
    pub fn with_priority(&mut self, repeated: bool, priority: ViewPriority) -> Duration {
        let active = self.observe(repeated);
        match priority {
            ViewPriority::Foreground => active,
            ViewPriority::Background => active.max(Duration::from_millis(200)),
        }
    }

    /// Return the desired start-to-start sampling interval after this record.
    /// For already slow profiles, preserve the original cadence.
    pub fn observe(&mut self, repeated: bool) -> Duration {
        if repeated {
            self.repeats = self.repeats.saturating_add(1);
        } else {
            self.repeats = 0;
        }
        let factor = match self.repeats {
            0..=3 => 1,
            4..=11 => 2,
            _ => 4,
        };
        self.base
            .saturating_mul(factor)
            .min(IDLE_LIMIT)
            .max(self.base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::VideoConfig;

    fn pacer(fps: u32) -> CapturePacer {
        CapturePacer::new(VideoConfig::new(540, fps, 75, 60).unwrap())
    }

    #[test]
    fn static_scene_slows_in_two_bounded_steps() {
        let mut pacing = pacer(30);
        let base = pacing.observe(false);
        for _ in 0..3 {
            assert_eq!(pacing.observe(true), base);
        }
        for _ in 4..12 {
            assert_eq!(pacing.observe(true), base * 2);
        }
        for _ in 0..300 {
            assert_eq!(pacing.observe(true), IDLE_LIMIT);
        }
        assert_eq!(pacing.observe(false), base);
        assert_eq!(pacing.observe(true), base);
    }

    #[test]
    fn slower_profiles_are_never_accelerated_or_delayed() {
        for fps in 1..=10 {
            let mut pacing = pacer(fps);
            let base = pacing.observe(false);
            for _ in 0..30 {
                assert_eq!(pacing.observe(true), base);
            }
        }
    }

    #[test]
    fn sixty_fps_and_background_cadence() {
        let mut pacing = pacer(60);
        let active = pacing.with_priority(false, ViewPriority::Foreground);
        assert_eq!(active, Duration::from_secs_f64(1.0 / 60.0));
        assert_eq!(
            pacing.with_priority(false, ViewPriority::Background),
            Duration::from_millis(200)
        );
        assert_eq!(
            pacing.with_priority(false, ViewPriority::Foreground),
            active
        );
        let mut slow = pacer(1);
        assert_eq!(
            slow.with_priority(false, ViewPriority::Background),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn intermittent_changes_do_not_enter_idle_mode() {
        let mut pacing = pacer(30);
        let base = pacing.observe(false);
        for _ in 0..100 {
            for _ in 0..3 {
                assert_eq!(pacing.observe(true), base);
            }
            assert_eq!(pacing.observe(false), base);
        }
    }
}
