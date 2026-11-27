//! Local sender policy: bounded work for dense motion and measured JPEG
//! payload feedback. These are soft targets, not wire limits or link estimates.
use crate::{desktop::VideoConfig, quality::MIN_JPEG_QUALITY};

const TARGET_BITS_PER_SECOND: usize = 24_000_000;
const OVERSIZE_STREAK: u8 = 3;
const RECOVERY_STREAK: u8 = 45;

pub(crate) struct MotionPolicy {
    ceiling: u8,
    jpeg_quality: u8,
    target: usize,
    expensive: u8,
    oversize: u8,
    small: u8,
}

impl MotionPolicy {
    pub(crate) fn new(config: VideoConfig) -> Self {
        Self {
            ceiling: config.quality(),
            jpeg_quality: config.quality(),
            target: TARGET_BITS_PER_SECOND / 8 / config.fps() as usize,
            expensive: 0,
            oversize: 0,
            small: 0,
        }
    }

    /// Sample a small fixed fraction of pixels, spread over the whole canvas.
    /// Heuristic only; never determines whether pixels are unchanged. Uniform
    /// fills and sparse text remain eligible for exact lossless encoding.
    pub(crate) fn dense(previous: &[u8], current: &[u8], width: usize, height: usize) -> bool {
        let mut samples = 0usize;
        let mut changed = 0usize;
        let mut detail = 0usize;
        for y in (4..height).step_by(16) {
            for x in (4..width.saturating_sub(4)).step_by(16) {
                let i = (y * width + x) * 3;
                let pixel = &current[i..i + 3];
                samples += 1;
                if pixel != &previous[i..i + 3] {
                    changed += 1;
                    let other = &current[i + 12..i + 15];
                    if pixel
                        .iter()
                        .zip(other)
                        .map(|(a, b)| u16::from(a.abs_diff(*b)))
                        .sum::<u16>()
                        >= 18
                    {
                        detail += 1;
                    }
                }
            }
        }
        samples != 0 && changed * 8 >= samples * 3 && detail * 4 >= samples
    }

    pub(crate) fn prefer_jpeg(&self, dense: bool) -> bool {
        dense && self.expensive >= 2
    }

    pub(crate) fn delta_budget(&self, budget: usize, dense: bool) -> usize {
        if dense {
            budget.min(self.target.saturating_mul(2).max(32 * 1024))
        } else {
            budget
        }
    }

    pub(crate) fn quality(&self, receipt_ceiling: u8, dense: bool) -> u8 {
        // The motion-only payload target must not permanently degrade a
        // later sparse UI keyframe. Receipt pressure still applies.
        if dense {
            self.jpeg_quality.min(receipt_ceiling)
        } else {
            receipt_ceiling
        }
    }

    pub(crate) fn committed(&mut self, jpeg: bool, bytes: usize, dense: bool) {
        if jpeg {
            self.expensive = self.expensive.saturating_add(1);
            if dense && bytes > self.target * 5 / 4 {
                self.small = 0;
                self.oversize += 1;
                if self.oversize == OVERSIZE_STREAK {
                    self.jpeg_quality = self.jpeg_quality.saturating_sub(10).max(MIN_JPEG_QUALITY);
                    self.oversize = 0;
                }
            } else if bytes <= self.target * 3 / 4 {
                self.oversize = 0;
                self.small += 1;
                if self.small == RECOVERY_STREAK {
                    self.jpeg_quality = self.jpeg_quality.saturating_add(5).min(self.ceiling);
                    self.small = 0;
                }
            } else {
                self.small = 0;
                self.oversize = 0;
            }
        } else {
            self.expensive = if dense && bytes > self.target {
                self.expensive.saturating_add(1)
            } else {
                0
            };
            self.oversize = 0;
            self.small = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> MotionPolicy {
        MotionPolicy::new(VideoConfig::new(1080, 60, 75, 15).unwrap())
    }

    #[test]
    fn measured_payload_feedback_is_bounded_and_recovers_slowly() {
        let mut p = policy();
        assert_eq!(p.target, 50_000);
        for _ in 0..2 {
            p.committed(true, 80_000, true);
        }
        assert!(p.prefer_jpeg(true));
        assert!(!p.prefer_jpeg(false));
        assert_eq!(p.quality(75, true), 75);
        p.committed(true, 80_000, true);
        assert_eq!(p.quality(75, true), 65);
        for _ in 0..100 {
            p.committed(true, 90_000, true);
        }
        assert_eq!(p.quality(75, true), 35);
        assert_eq!(p.quality(75, false), 75);
        for _ in 0..44 {
            p.committed(true, 20_000, true);
        }
        assert_eq!(p.quality(75, true), 35);
        p.committed(true, 20_000, true);
        assert_eq!(p.quality(75, true), 40);
        for _ in 0..500 {
            p.committed(true, 20_000, true);
        }
        assert_eq!(p.quality(75, true), 75);
        assert_eq!(p.quality(45, true), 45);
        p.committed(false, 100, false);
        assert!(!p.prefer_jpeg(true));
        assert_eq!(p.delta_budget(512 * 1024, true), 100_000);
        assert_eq!(p.delta_budget(512 * 1024, false), 512 * 1024);
    }

    #[test]
    fn sample_distinguishes_sparse_and_uniform_from_textured_motion() {
        let (w, h) = (960, 540);
        let old = vec![0; w * h * 3];
        let mut frame = vec![50; w * h * 3];
        assert!(!MotionPolicy::dense(&old, &frame, w, h));
        for (i, pixel) in frame.chunks_exact_mut(3).enumerate() {
            let x = i % w;
            pixel.copy_from_slice(&[(x * 7) as u8, (x * 13 + i / w) as u8, ((i / w) * 17) as u8]);
        }
        assert!(MotionPolicy::dense(&old, &frame, w, h));
        assert!(!MotionPolicy::dense(&frame, &frame, w, h));
        let mut small = old.clone();
        small[..w * 3 * 20].fill(200);
        assert!(!MotionPolicy::dense(&old, &small, w, h));
    }
}
