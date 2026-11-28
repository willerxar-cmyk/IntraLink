//! A no-update observation is distinct from verified identical pixels.
//! Delay the visible notice to avoid flicker between normal frame arrivals.
use intralink_core::desktop::Record;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

const NOTICE_DELAY_MS: u64 = 1_000;

pub(super) struct SourceStatus {
    origin: Instant,
    since: AtomicU64,
    #[cfg(all(target_os = "windows", feature = "input-preview"))]
    sample: AtomicU64,
}

impl Default for SourceStatus {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            since: AtomicU64::new(0),
            #[cfg(all(target_os = "windows", feature = "input-preview"))]
            sample: AtomicU64::new(0),
        }
    }
}

impl SourceStatus {
    fn elapsed_ms(&self) -> u64 {
        self.origin
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX - 1)) as u64
    }

    pub(super) fn observe(&self, record: Record) {
        self.observe_at(record, self.elapsed_ms());
    }

    fn observe_at(&self, record: Record, ms: u64) {
        #[cfg(all(target_os = "windows", feature = "input-preview"))]
        match record {
            Record::Frame(_) | Record::Repeat => {
                self.sample.store(ms.saturating_add(1), Ordering::Release)
            }
            Record::End => self.sample.store(0, Ordering::Release),
            Record::NoUpdate => {}
        }
        if record == Record::NoUpdate {
            // Zero means no active no-update interval. Preserve the first
            // observation rather than extending the deadline on every poll.
            let _ = self.since.compare_exchange(
                0,
                ms.saturating_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        } else {
            self.since.store(0, Ordering::Release);
        }
    }

    /// Latest received frame or verified repeat observation. Not a remote
    /// timestamp or proof that the local presentation has zero frame age.
    #[cfg(all(target_os = "windows", feature = "input-preview"))]
    pub(super) fn sample_recent(&self) -> bool {
        self.sample_recent_at(self.elapsed_ms())
    }

    #[cfg(all(target_os = "windows", feature = "input-preview"))]
    fn sample_recent_at(&self, ms: u64) -> bool {
        let last = self.sample.load(Ordering::Acquire);
        last != 0 && ms.saturating_sub(last - 1) <= 2000
    }

    pub(super) fn notice_due(&self) -> bool {
        self.notice_due_at(self.elapsed_ms())
    }

    fn notice_due_at(&self, ms: u64) -> bool {
        let since = self.since.load(Ordering::Acquire);
        since != 0
            && Duration::from_millis(ms.saturating_sub(since - 1))
                >= Duration::from_millis(NOTICE_DELAY_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_os = "windows", feature = "input-preview"))]
    #[test]
    fn only_frame_and_repeat_refresh_direct_control_sample_window() {
        let state = SourceStatus::default();
        assert!(!state.sample_recent_at(0));
        state.observe_at(Record::Frame(4), 100);
        assert!(state.sample_recent_at(2100));
        state.observe_at(Record::NoUpdate, 2000);
        assert!(!state.sample_recent_at(2101));
        state.observe_at(Record::Repeat, 2200);
        assert!(state.sample_recent_at(4200));
        state.observe_at(Record::End, 4201);
        assert!(!state.sample_recent_at(4201));
    }

    #[test]
    fn no_update_notice_is_delayed_and_only_observed_pixels_clear_it() {
        let state = SourceStatus::default();
        assert!(!state.notice_due_at(10_000));
        state.observe_at(Record::NoUpdate, 100);
        state.observe_at(Record::NoUpdate, 500);
        assert!(!state.notice_due_at(1099));
        assert!(state.notice_due_at(1100));
        state.observe_at(Record::Repeat, 1200);
        assert!(!state.notice_due_at(2000));
        state.observe_at(Record::NoUpdate, 2100);
        assert!(!state.notice_due_at(3000));
        assert!(state.notice_due_at(3100));
        state.observe_at(Record::Frame(4), 3200);
        assert!(!state.notice_due_at(5000));
    }
}
