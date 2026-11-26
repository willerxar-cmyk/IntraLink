//! Per-session local media counters and rate sampling. No telemetry leaves the device.
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

#[derive(Debug, Default)]
pub struct SessionCounters {
    received: AtomicU64,
    decoded: AtomicU64,
    presented: AtomicU64,
    media_bytes: AtomicU64,
    decode_ns: AtomicU64,
    present_ns: AtomicU64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counters {
    pub received: u64,
    pub decoded: u64,
    pub presented: u64,
    pub media_bytes: u64,
    pub decode_ns: u64,
    pub present_ns: u64,
}

fn nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

impl SessionCounters {
    /// Complete changed records, before the bounded decoder queue. Repeat
    /// records are intentionally excluded, as are QUIC headers and ACKs.
    pub fn received(&self, payload_bytes: usize) {
        self.media_bytes
            .fetch_add(payload_bytes as u64, Ordering::Relaxed);
        self.received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decoded(&self, elapsed: Duration) {
        self.decode_ns.fetch_add(nanos(elapsed), Ordering::Relaxed);
        self.decoded.fetch_add(1, Ordering::Relaxed);
    }

    /// Call only after the native presenter accepts a new sequence. Redraws
    /// of the same sequence and overview thumbnails are not new presentations.
    pub fn presented(&self, elapsed: Duration) {
        self.present_ns.fetch_add(nanos(elapsed), Ordering::Relaxed);
        self.presented.fetch_add(1, Ordering::Relaxed);
    }

    /// Independent observational counters, not a synchronization barrier.
    /// Do not infer lost frames from transient differences between stages.
    pub fn snapshot(&self) -> Counters {
        Counters {
            received: self.received.load(Ordering::Relaxed),
            decoded: self.decoded.load(Ordering::Relaxed),
            presented: self.presented.load(Ordering::Relaxed),
            media_bytes: self.media_bytes.load(Ordering::Relaxed),
            decode_ns: self.decode_ns.load(Ordering::Relaxed),
            present_ns: self.present_ns.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Rates {
    pub received_fps: f64,
    pub decoded_fps: f64,
    pub presented_fps: f64,
    /// Changed media payload only; not the full network bitrate.
    pub payload_mbps: f64,
    pub mean_decode_ms: Option<f64>,
    pub mean_present_ms: Option<f64>,
}

#[derive(Debug)]
pub struct RateSampler {
    previous: Counters,
    sampled_at: Duration,
    rates: Option<Rates>,
}

impl RateSampler {
    pub fn new(now: Duration, counters: Counters) -> Self {
        Self {
            previous: counters,
            sampled_at: now,
            rates: None,
        }
    }

    pub fn latest(&self) -> Option<Rates> {
        self.rates
    }

    /// Measure completed work over actual elapsed monotonic time. Do not
    /// extrapolate a partial interval or reuse rates after a counter reset.
    pub fn sample(&mut self, now: Duration, current: Counters) -> Option<Rates> {
        let old = self.previous;
        let reset = current.received < old.received
            || current.decoded < old.decoded
            || current.presented < old.presented
            || current.media_bytes < old.media_bytes
            || current.decode_ns < old.decode_ns
            || current.present_ns < old.present_ns;
        if reset || now < self.sampled_at {
            *self = Self::new(now, current);
            return None;
        }
        let elapsed = now - self.sampled_at;
        if elapsed < Duration::from_secs(1) {
            return self.rates;
        }
        let seconds = elapsed.as_secs_f64();
        let decoded = current.decoded - old.decoded;
        let presented = current.presented - old.presented;
        let rates = Rates {
            received_fps: (current.received - old.received) as f64 / seconds,
            decoded_fps: decoded as f64 / seconds,
            presented_fps: presented as f64 / seconds,
            payload_mbps: (current.media_bytes - old.media_bytes) as f64 * 8.0
                / 1_000_000.0
                / seconds,
            mean_decode_ms: (decoded > 0)
                .then(|| (current.decode_ns - old.decode_ns) as f64 / decoded as f64 / 1_000_000.0),
            mean_present_ms: (presented > 0).then(|| {
                (current.present_ns - old.present_ns) as f64 / presented as f64 / 1_000_000.0
            }),
        };
        self.previous = current;
        self.sampled_at = now;
        self.rates = Some(rates);
        self.rates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_are_independent_and_payload_is_counted_once() {
        let counters = SessionCounters::default();
        counters.received(1500);
        counters.received(2000);
        counters.decoded(Duration::from_millis(3));
        counters.presented(Duration::from_millis(1));
        assert_eq!(
            counters.snapshot(),
            Counters {
                received: 2,
                decoded: 1,
                presented: 1,
                media_bytes: 3500,
                decode_ns: 3_000_000,
                present_ns: 1_000_000,
            }
        );
    }

    #[test]
    fn actual_elapsed_time_and_distinct_rates() {
        let mut sampler = RateSampler::new(Duration::ZERO, Counters::default());
        let totals = Counters {
            received: 120,
            decoded: 118,
            presented: 100,
            media_bytes: 3_000_000,
            decode_ns: 236_000_000,
            present_ns: 100_000_000,
        };
        assert!(sampler.sample(Duration::from_millis(999), totals).is_none());
        let rates = sampler.sample(Duration::from_secs(2), totals).unwrap();
        assert_eq!(rates.received_fps, 60.0);
        assert_eq!(rates.decoded_fps, 59.0);
        assert_eq!(rates.presented_fps, 50.0);
        assert_eq!(rates.payload_mbps, 12.0);
        assert_eq!(rates.mean_decode_ms, Some(2.0));
        assert_eq!(rates.mean_present_ms, Some(1.0));
        assert_eq!(sampler.latest(), Some(rates));
        assert_eq!(
            sampler.sample(Duration::from_millis(2500), totals),
            Some(rates)
        );
        let idle = sampler.sample(Duration::from_secs(3), totals).unwrap();
        assert_eq!(idle, Rates::default());
    }

    #[test]
    fn resets_never_report_old_session_rates() {
        let mut sampler = RateSampler::new(Duration::ZERO, Counters::default());
        let current = Counters {
            received: 60,
            ..Counters::default()
        };
        assert!(sampler.sample(Duration::from_secs(1), current).is_some());
        assert!(
            sampler
                .sample(Duration::from_secs(1), Counters::default())
                .is_none()
        );
        assert!(sampler.latest().is_none());
        assert!(
            sampler
                .sample(Duration::ZERO, Counters::default())
                .is_none()
        );
    }

    #[test]
    fn concurrent_updates_are_not_lost() {
        let counters = SessionCounters::default();
        std::thread::scope(|scope| {
            for _ in 0..3 {
                scope.spawn(|| {
                    for _ in 0..1000 {
                        counters.received(100);
                        counters.decoded(Duration::from_micros(500));
                        counters.presented(Duration::from_micros(100));
                    }
                });
            }
        });
        let totals = counters.snapshot();
        assert_eq!(totals.received, 3000);
        assert_eq!(totals.decoded, 3000);
        assert_eq!(totals.presented, 3000);
        assert_eq!(totals.media_bytes, 300_000);
        assert_eq!(totals.decode_ns, 1_500_000_000);
        assert_eq!(totals.present_ns, 300_000_000);
    }
}
