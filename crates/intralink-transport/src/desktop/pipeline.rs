//! Opt-in local two-demand window for fixed v8 desktop sessions.
//! Authentication/handshake, wire records and receipt drain are shared.
use anyhow::{Context, Result, bail, ensure};
use intralink_core::{
    desktop::{
        CreditWindow, MAX_FRAME_BYTES, MAX_IN_FLIGHT_RECORDS, NO_UPDATE, REPEAT, VideoConfig,
        ViewPriority, frame_header,
    },
    pacing::CapturePacer,
    quality::ReceiptPressure,
};
use iroh::endpoint::SendStream;
use std::{collections::VecDeque, time::Duration};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, sleep_until, timeout},
};

use super::{Capture, CapturePermit, CaptureRequest, IO_LIMIT, Stats};

const LOCAL_WINDOW: usize = 2;
const BACKGROUND_INTERVAL: Duration = Duration::from_millis(200);

struct Pending {
    response: oneshot::Receiver<Result<Capture>>,
    expires: Instant,
    started: bool,
}

struct AwaitStart {
    sequence: usize,
    not_before: Instant,
    expires: Instant,
    reply: oneshot::Receiver<std::time::Instant>,
}

fn may_issue(
    config: VideoConfig,
    stats: &Stats,
    credit: &CreditWindow,
    pending: usize,
    last_changed: bool,
) -> bool {
    pending < LOCAL_WINDOW
        && stats.records() + pending < config.max_frames()
        && credit.outstanding() + pending < MAX_IN_FLIGHT_RECORDS
        && credit.has_credit()
        && (pending == 0
            || (last_changed && stats.frames > 0 && credit.priority() == ViewPriority::Foreground))
}

/// Send only in FIFO demand order. Each unsent demand reserves one of the
/// existing eight credit positions; no speculative capture or extra wire credit.
/// At most one already issued demand can precede newly observed idle/priority
/// feedback. After that observation no further overlapping demand is issued.
pub(super) async fn stream(
    tx: &mut SendStream,
    receipts: &mut mpsc::Receiver<[u8; 4]>,
    source: &mpsc::Sender<CaptureRequest>,
    config: VideoConfig,
    deadline: Instant,
) -> Result<(Stats, CreditWindow)> {
    let mut stats = Stats::default();
    let mut credit = CreditWindow::default();
    let mut pacer = CapturePacer::new(config);
    let mut interval = pacer.observe(false);
    let mut last_started = None::<Instant>;
    let mut awaiting_start = None::<AwaitStart>;
    let mut last_changed = false;
    let mut pending = VecDeque::<Pending>::with_capacity(LOCAL_WINDOW);
    let mut buffers = Vec::<Vec<u8>>::with_capacity(LOCAL_WINDOW);

    while stats.records() < config.max_frames() && Instant::now() < deadline {
        while let Ok(bytes) = receipts.try_recv() {
            ensure!(credit.acknowledge(bytes), "Invalid frame acknowledgment");
        }
        let now = Instant::now();
        let cadence = if credit.priority() == ViewPriority::Background {
            interval.max(BACKGROUND_INTERVAL)
        } else {
            interval
        };
        // Reserve early, but the source cannot start until this exact
        // monotonic deadline. Base future deadlines on actual starts, not
        // the timing of the transport request or the preceding response.
        let due = last_started.map_or(now, |last| (last + cadence).max(now));
        let allowed = awaiting_start.is_none()
            && due < deadline
            && may_issue(config, &stats, &credit, pending.len(), last_changed);
        let has_pending = !pending.is_empty();
        let expires = pending
            .front()
            .map_or(now + IO_LIMIT, |request| request.expires);
        let has_start = awaiting_start.is_some();
        tokio::select! {
            biased;
            _ = sleep_until(deadline) => break,
            _ = sleep_until(expires), if has_pending => bail!("Local pipelined capture timed out"),
            received = receipts.recv() => {
                let bytes = received.context("Desktop receipt stream closed")?;
                ensure!(credit.acknowledge(bytes), "Invalid frame acknowledgment");
            }
            reported = async {
                (&mut awaiting_start.as_mut().expect("start branch enabled").reply).await
            }, if has_start => {
                let grant = awaiting_start.take().context("Missing capture start grant")?;
                let actual = Instant::from_std(reported.context("Local capture never started")?);
                ensure!(actual >= grant.not_before && actual < grant.expires && actual <= Instant::now(),
                    "Invalid reported capture start");
                ensure!(stats.records() + pending.len() == grant.sequence, "Capture start sequence mismatch");
                let request = pending.back_mut().context("Missing started capture demand")?;
                ensure!(!request.started, "Capture demand started twice");
                request.started = true;
                last_started = Some(actual);
            }
            response = async {
                (&mut pending.front_mut().expect("pending branch enabled").response).await
            }, if has_pending => {
                let frame = response.context("Local pipelined capture response dropped")??;
                let request = pending.pop_front().context("Missing ordered demand")?;
                ensure!(request.started, "Capture replied without starting its reserved grant");
                if Instant::now() >= deadline { break; }
                let (header, mut data, changed, no_update) = match frame {
                    Capture::End => break,
                    Capture::Repeat(data) => {
                        ensure!(stats.frames > 0 && data.is_empty(), "Invalid local repeat");
                        (REPEAT, data, false, false)
                    }
                    Capture::NoUpdate(data) => {
                        ensure!(stats.frames > 0 && data.is_empty(), "Invalid local no-update");
                        (NO_UPDATE, data, false, true)
                    }
                    Capture::Frame(data) => (
                        frame_header(data.len()).context("Invalid local frame length")?,
                        data, true, false,
                    ),
                    Capture::AdaptiveFrame { .. } => bail!("Adaptive record not permitted in fixed pipeline"),
                };
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() { break; }
                timeout(IO_LIMIT.min(remaining), async {
                    tx.write_all(&header).await?;
                    if changed { tx.write_all(&data).await?; }
                    Ok::<(), anyhow::Error>(())
                }).await.context("Desktop transfer timed out")??;
                ensure!(credit.record_sent(), "Frame credit exhausted");
                if changed {
                    stats.frames += 1;
                    stats.media_bytes += data.len() as u64;
                } else if no_update {
                    stats.no_updates += 1;
                } else {
                    stats.repeats += 1;
                }
                last_changed = changed;
                interval = pacer.observe(!changed);
                // Return at most two bounded transport allocations. The
                // producer owns each buffer until the ordered response.
                data.clear();
                if data.capacity() <= MAX_FRAME_BYTES && buffers.len() < LOCAL_WINDOW {
                    buffers.push(data);
                }
            }
            permit = source.reserve(), if allowed => {
                let permit = permit.context("Local capture source stopped")?;
                let issued = Instant::now();
                let not_before = due.max(issued);
                let expires = (issued + IO_LIMIT).min(deadline);
                if not_before >= expires { break; }
                let (reply, response) = oneshot::channel();
                let (started, start_reply) = oneshot::channel();
                permit.send(CaptureRequest {
                    buffer: buffers.pop().unwrap_or_default(),
                    reply,
                    receipt_pressure: ReceiptPressure::from_backlog(
                        credit.outstanding() + pending.len(), credit.priority()),
                    desktop_config: Some(config),
                    issued_at: issued.into_std(),
                    permit: Some(CapturePermit {
                        not_before: not_before.into_std(),
                        expires: expires.into_std(),
                        started: Some(started),
                    }),
                });
                pending.push_back(Pending { response, expires, started: false });
                awaiting_start = Some(AwaitStart {
                    sequence: stats.records() + pending.len(),
                    not_before,
                    expires,
                    reply: start_reply,
                });
            }
            // No in-flight demand and no possible new demand: wait for
            // credit, but never extend a stalled connection indefinitely.
            _ = sleep_until(expires), if !has_pending => bail!("Desktop receipt window stalled"),
        }
    }
    // Cancel all reserved but unsent demands before final receipt drain.
    drop(awaiting_start);
    drop(pending);
    Ok((stats, credit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use intralink_core::desktop::{ack, ack_with_priority};

    #[test]
    fn reservations_share_existing_credit_and_record_limit() {
        let config = VideoConfig::new(540, 10, 75, 1).unwrap();
        let mut stats = Stats {
            frames: 1,
            ..Stats::default()
        };
        let mut credit = CreditWindow::default();
        assert!(may_issue(config, &stats, &credit, 1, true));
        assert!(!may_issue(config, &stats, &credit, 2, true));
        assert!(!may_issue(config, &stats, &credit, 1, false));
        for _ in 0..7 {
            assert!(credit.record_sent());
        }
        assert!(may_issue(config, &stats, &credit, 0, true));
        assert!(!may_issue(config, &stats, &credit, 1, true));
        assert!(credit.acknowledge(ack(1).unwrap()));
        assert!(may_issue(config, &stats, &credit, 1, true));
        assert!(credit.acknowledge(ack_with_priority(2, ViewPriority::Background).unwrap()));
        assert!(!may_issue(config, &stats, &credit, 1, true));
        assert!(may_issue(config, &stats, &credit, 0, true));
        stats.frames = config.max_frames() - 1;
        assert!(!may_issue(config, &stats, &credit, 1, true));
    }

    #[test]
    fn first_keyframe_cannot_have_overlapping_demand() {
        let config = VideoConfig::new(540, 60, 75, 1).unwrap();
        let stats = Stats::default();
        let credit = CreditWindow::default();
        assert!(may_issue(config, &stats, &credit, 0, false));
        assert!(!may_issue(config, &stats, &credit, 1, true));
    }
}
