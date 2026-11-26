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
}

impl Default for SourceStatus {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            since: AtomicU64::new(0),
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
