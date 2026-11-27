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

use super::{Capture, CaptureRequest, IO_LIMIT, Stats};

const LOCAL_WINDOW: usize = 2;
const BACKGROUND_INTERVAL: Duration = Duration::from_millis(200);

struct Pending {
    response: oneshot::Receiver<Result<Capture>>,
    expires: Instant,
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
    let mut last_issued = None::<Instant>;
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
        let due = last_issued.map_or(now, |last| last + cadence);
        let allowed = may_issue(config, &stats, &credit, pending.len(), last_changed);
        let has_pending = !pending.is_empty();
        let expires = pending
            .front()
            .map_or(now + IO_LIMIT, |request| request.expires);
        let wait_due = due.min(deadline);
        tokio::select! {
            biased;
            _ = sleep_until(deadline) => break,
            _ = sleep_until(expires), if has_pending => bail!("Local pipelined capture timed out"),
            received = receipts.recv() => {
                let bytes = received.context("Desktop receipt stream closed")?;
                ensure!(credit.acknowledge(bytes), "Invalid frame acknowledgment");
            }
            response = async {
                (&mut pending.front_mut().expect("pending branch enabled").response).await
            }, if has_pending => {
                let frame = response.context("Local pipelined capture response dropped")??;
                pending.pop_front().context("Missing ordered demand")?;
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
            permit = source.reserve(), if allowed && now >= due => {
                let permit = permit.context("Local capture source stopped")?;
                let issued = Instant::now();
                if issued >= deadline { break; }
                let (reply, response) = oneshot::channel();
                permit.send(CaptureRequest {
                    buffer: buffers.pop().unwrap_or_default(),
                    reply,
                    receipt_pressure: ReceiptPressure::from_backlog(
                        credit.outstanding() + pending.len(), credit.priority()),
                    desktop_config: Some(config),
                });
                pending.push_back(Pending { response, expires: issued + IO_LIMIT });
                last_issued = Some(issued);
            }
            _ = sleep_until(wait_due), if allowed && now < due => {}
            // No in-flight demand and no possible new demand: wait for
            // credit, but never extend a stalled connection indefinitely.
            _ = sleep_until(expires), if !has_pending => bail!("Desktop receipt window stalled"),
        }
    }
    // Cancel all reserved but unsent demands before final receipt drain.
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
