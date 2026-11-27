//! Local push-to-demand bridge for the existing strict v9 AVC diagnostic.
//! Never assigns a wire sequence before a validated record is requested.
use super::{Error, Framing, MAX_ACCESS_UNIT, Packetizer, Profile, inspect_nal_types};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub const MAX_PENDING_PICTURES: usize = 2;
pub const MAX_PENDING_BYTES: usize = MAX_PENDING_PICTURES * MAX_ACCESS_UNIT;
const MAX_AGE: Duration = Duration::from_millis(150);
const RECOVERY_LIMIT: Duration = Duration::from_secs(2);
const REQUEST_INTERVAL: Duration = Duration::from_millis(250);
const IDR: u32 = 1 << 5;
const VCL: u32 = 0x3e;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProducerError {
    Closed,
    Clock,
    Timestamp,
    RecoveryExpired,
    Codec(Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Push {
    Queued,
    Dropped { request_keyframe: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poll {
    Frame,
    NoUpdate { request_keyframe: bool },
}

struct Picture {
    bytes: Box<[u8]>,
    framing: Framing,
    key: bool,
    arrived: Instant,
}

/// A single locally approved encoder epoch, owned by one scheduler.
/// Accept owned, exact-size allocations; at most two pending coded pictures.
/// Wire credit, pacing, peer approval and cancellation remain the transport's
/// responsibility. Loss is signaled explicitly; never drop a P picture and
/// continue sending subsequent dependent pictures.
pub struct Producer {
    packetizer: Packetizer,
    pending: VecDeque<Picture>,
    require_idr: bool,
    recovery_started: Option<Instant>,
    last_request: Option<Instant>,
    last_pts: Option<i64>,
    last_now: Instant,
    closed: bool,
}

impl Producer {
    pub fn new(profile: Profile, now: Instant) -> Self {
        Self {
            packetizer: Packetizer::new(profile),
            pending: VecDeque::with_capacity(MAX_PENDING_PICTURES),
            require_idr: true,
            recovery_started: Some(now),
            last_request: None,
            last_pts: None,
            last_now: now,
            closed: false,
        }
    }

    pub fn pending_bytes(&self) -> usize {
        self.pending.iter().map(|p| p.bytes.len()).sum()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn close(&mut self) {
        self.pending.clear();
        self.closed = true;
    }

    fn fail<T>(&mut self, error: ProducerError) -> Result<T, ProducerError> {
        self.close();
        Err(error)
    }

    fn tick(&mut self, now: Instant) -> Result<(), ProducerError> {
        if self.closed {
            return Err(ProducerError::Closed);
        }
        if now < self.last_now {
            return self.fail(ProducerError::Clock);
        }
        self.last_now = now;
        if self
            .recovery_started
            .is_some_and(|start| now.duration_since(start) >= RECOVERY_LIMIT)
        {
            return self.fail(ProducerError::RecoveryExpired);
        }
        Ok(())
    }

    fn lose(&mut self, now: Instant) {
        self.pending.clear();
        self.require_idr = true;
        self.recovery_started.get_or_insert(now);
    }

    fn request(&mut self, now: Instant) -> bool {
        if !self.require_idr
            || self
                .last_request
                .is_some_and(|last| now.duration_since(last) < REQUEST_INTERVAL)
        {
            return false;
        }
        self.last_request = Some(now);
        true
    }

    /// Separate local CSD. The strict v9 parameter validator still applies;
    /// changing the approved codec configuration requires a new session.
    pub fn configure(
        &mut self,
        framing: Framing,
        sps: &[u8],
        pps: &[u8],
        now: Instant,
    ) -> Result<(), ProducerError> {
        self.tick(now)?;
        if let Err(error) = self.packetizer.configure(framing, sps, pps) {
            return self.fail(ProducerError::Codec(error));
        }
        Ok(())
    }

    /// Notify a lost, partial or discarded encoder picture, including loss
    /// before this mailbox. The owner should request a fresh IDR if indicated.
    pub fn notify_loss(&mut self, now: Instant) -> Result<bool, ProducerError> {
        self.tick(now)?;
        self.lose(now);
        Ok(self.request(now))
    }

    /// One complete local access unit with its actual encoder timestamp.
    /// No inferred framing or synthetic timestamp. A malformed buffer closes
    /// this epoch. A queued header is not proof of a complete coded picture;
    /// the strict packetizer validates it before any record leaves the queue.
    pub fn push(
        &mut self,
        framing: Framing,
        key: bool,
        pts_us: i64,
        bytes: Box<[u8]>,
        now: Instant,
    ) -> Result<Push, ProducerError> {
        self.tick(now)?;
        if pts_us < 0 || self.last_pts.is_some_and(|last| pts_us <= last) {
            return self.fail(ProducerError::Timestamp);
        }
        self.last_pts = Some(pts_us);
        let mask = match inspect_nal_types(framing, &bytes) {
            Ok(mask) => mask,
            Err(error) => return self.fail(ProducerError::Codec(error)),
        };
        let vcl = mask & VCL;
        if !matches!(vcl, 2 | IDR) || key != (vcl == IDR) {
            return self.fail(ProducerError::Codec(Error::Keyframe));
        }
        if self
            .pending
            .front()
            .is_some_and(|p| now.duration_since(p.arrived) >= MAX_AGE)
            || self.pending.len() == MAX_PENDING_PICTURES
        {
            self.lose(now);
        }
        if self.require_idr && !key {
            return Ok(Push::Dropped {
                request_keyframe: self.request(now),
            });
        }
        if key {
            self.require_idr = false;
        }
        self.pending.push_back(Picture {
            bytes,
            framing,
            key,
            arrived: now,
        });
        debug_assert!(self.pending_bytes() <= MAX_PENDING_BYTES);
        Ok(Push::Queued)
    }

    /// Consume one actual transport demand into its recycled output buffer.
    /// A loss clears all unsent references; previously issued records remain
    /// ordered in the existing QUIC stream. Only a validated IDR can resume
    /// after loss. Never synthesize a repeat or skip a wire sequence.
    pub fn poll(&mut self, now: Instant, output: &mut Vec<u8>) -> Result<Poll, ProducerError> {
        output.clear();
        self.tick(now)?;
        if self
            .pending
            .front()
            .is_some_and(|p| now.duration_since(p.arrived) >= MAX_AGE)
        {
            self.lose(now);
        }
        let Some(picture) = self.pending.pop_front() else {
            return Ok(Poll::NoUpdate {
                request_keyframe: self.request(now),
            });
        };
        match self
            .packetizer
            .packetize(picture.framing, picture.key, &picture.bytes, output)
        {
            Ok(()) => {
                if picture.key {
                    self.recovery_started = None;
                    self.last_request = None;
                }
                Ok(Poll::Frame)
            }
            Err(Error::Gop) => {
                self.lose(now);
                Ok(Poll::NoUpdate {
                    request_keyframe: self.request(now),
                })
            }
            Err(error) => self.fail(ProducerError::Codec(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::h264::{
        StreamState,
        tests::{IDR as KEY, P},
        unwrap,
    };

    fn profile() -> Profile {
        Profile::new(540, 3, 10, 8_000_000).unwrap()
    }

    fn push(
        source: &mut Producer,
        key: bool,
        pts: i64,
        now: Instant,
    ) -> Result<Push, ProducerError> {
        source.push(
            Framing::AnnexB,
            key,
            pts,
            if key { KEY } else { P }.into(),
            now,
        )
    }

    #[test]
    fn pressure_drops_entire_unsent_chain_and_resumes_at_idr() {
        let start = Instant::now();
        let mut source = Producer::new(profile(), start);
        let mut receiver = StreamState::new(profile());
        let mut output = Vec::new();
        assert_eq!(push(&mut source, true, 1, start), Ok(Push::Queued));
        assert_eq!(source.poll(start, &mut output), Ok(Poll::Frame));
        assert_eq!(receiver.accept(&output).unwrap().sequence, 1);
        for pts in 2..=3 {
            assert_eq!(push(&mut source, false, pts, start), Ok(Push::Queued));
        }
        assert_eq!(source.pending_count(), MAX_PENDING_PICTURES);
        assert_eq!(
            push(&mut source, false, 4, start),
            Ok(Push::Dropped {
                request_keyframe: true
            })
        );
        assert_eq!(source.pending_bytes(), 0);
        assert_eq!(
            push(&mut source, false, 5, start),
            Ok(Push::Dropped {
                request_keyframe: false
            })
        );
        assert_eq!(
            source.poll(start, &mut output),
            Ok(Poll::NoUpdate {
                request_keyframe: false
            })
        );
        assert!(output.is_empty());
        assert_eq!(push(&mut source, true, 6, start), Ok(Push::Queued));
        assert_eq!(push(&mut source, false, 7, start), Ok(Push::Queued));
        assert_eq!(source.poll(start, &mut output), Ok(Poll::Frame));
        assert_eq!(receiver.accept(&output).unwrap().sequence, 2);
        assert!(unwrap(&output).unwrap().keyframe);
        assert_eq!(source.poll(start, &mut output), Ok(Poll::Frame));
        assert_eq!(receiver.accept(&output).unwrap().sequence, 3);
    }

    #[test]
    fn stale_picture_loss_requires_new_idr_and_deadline_cannot_be_extended() {
        let start = Instant::now();
        let mut source = Producer::new(profile(), start);
        let mut output = Vec::new();
        assert_eq!(push(&mut source, true, 1, start), Ok(Push::Queued));
        let stale = start + MAX_AGE;
        assert_eq!(
            source.poll(stale, &mut output),
            Ok(Poll::NoUpdate {
                request_keyframe: true
            })
        );
        assert_eq!(source.pending_count(), 0);
        assert_eq!(source.notify_loss(stale + REQUEST_INTERVAL), Ok(true));
        assert_eq!(
            source.poll(start + RECOVERY_LIMIT, &mut output),
            Err(ProducerError::RecoveryExpired)
        );
        assert!(output.is_empty());
        assert_eq!(source.pending_bytes(), 0);
        assert_eq!(
            push(&mut source, true, 2, start + RECOVERY_LIMIT),
            Err(ProducerError::Closed)
        );
    }

    #[test]
    fn actual_loss_and_gop_expiry_do_not_emit_dependent_records() {
        let start = Instant::now();
        let mut source = Producer::new(profile(), start);
        let mut output = Vec::new();
        assert_eq!(
            push(&mut source, false, 1, start),
            Ok(Push::Dropped {
                request_keyframe: true
            })
        );
        assert_eq!(push(&mut source, true, 2, start), Ok(Push::Queued));
        assert_eq!(source.poll(start, &mut output), Ok(Poll::Frame));
        for pts in 3..=4 {
            assert_eq!(push(&mut source, false, pts, start), Ok(Push::Queued));
            assert_eq!(source.poll(start, &mut output), Ok(Poll::Frame));
        }
        assert_eq!(push(&mut source, false, 5, start), Ok(Push::Queued));
        assert_eq!(
            source.poll(start, &mut output),
            Ok(Poll::NoUpdate {
                request_keyframe: true
            })
        );
        assert!(output.is_empty());
        assert_eq!(push(&mut source, true, 6, start), Ok(Push::Queued));
        assert_eq!(source.notify_loss(start), Ok(false));
        assert_eq!(source.pending_count(), 0);
        assert_eq!(
            push(&mut source, false, 7, start),
            Ok(Push::Dropped {
                request_keyframe: false
            })
        );
        assert_eq!(push(&mut source, true, 8, start), Ok(Push::Queued));
        assert_eq!(source.poll(start, &mut output), Ok(Poll::Frame));
        assert_eq!(unwrap(&output).unwrap().sequence, 4);
    }

    #[test]
    fn malformed_or_reordered_input_closes_epoch_and_discards_pending() {
        let start = Instant::now();
        let mut source = Producer::new(profile(), start);
        assert_eq!(push(&mut source, true, 2, start), Ok(Push::Queued));
        assert_eq!(
            push(&mut source, false, 2, start),
            Err(ProducerError::Timestamp)
        );
        assert_eq!(source.pending_count(), 0);
        let mut output = vec![1];
        assert_eq!(source.poll(start, &mut output), Err(ProducerError::Closed));
        assert!(output.is_empty());

        let mut source = Producer::new(profile(), start);
        assert_eq!(
            source.push(Framing::AnnexB, true, 0, P.into(), start),
            Err(ProducerError::Codec(Error::Keyframe))
        );
        let mut source = Producer::new(profile(), start);
        assert_eq!(
            source.push(
                Framing::AnnexB,
                true,
                0,
                vec![1; MAX_ACCESS_UNIT + 1].into_boxed_slice(),
                start
            ),
            Err(ProducerError::Codec(Error::Size))
        );
        let mut source = Producer::new(profile(), start);
        assert_eq!(
            source.poll(start - Duration::from_millis(1), &mut output),
            Err(ProducerError::Clock)
        );
    }

    #[test]
    fn incompatible_keyframe_cannot_escape_queue() {
        let start = Instant::now();
        let mut source = Producer::new(profile(), start);
        let mut output = Vec::new();
        // Header can identify IDR, but there is no validated SPS/PPS.
        assert_eq!(
            source.push(
                Framing::AnnexB,
                true,
                1,
                [0, 0, 1, 0x65, 0xb8].into(),
                start
            ),
            Ok(Push::Queued)
        );
        assert_eq!(
            source.poll(start, &mut output),
            Err(ProducerError::Codec(Error::ParameterSets))
        );
        assert!(output.is_empty());
        assert_eq!(source.poll(start, &mut output), Err(ProducerError::Closed));
    }
}
