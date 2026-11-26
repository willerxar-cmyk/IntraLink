//! Continuous WGC -> authenticated QUIC -> GPU preview on loopback only.
//! A separate local trusted packet channel prevents any arbitrary wire bytes
//! from reaching the native NVIDIA decoder. No physical desktop or vault.
use std::{
    cell::RefCell,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use intralink_core::{
    desktop::MAX_IN_FLIGHT_RECORDS,
    h264::{self, Profile},
};
use intralink_d3d11::{
    FIXTURE_BITRATE, FIXTURE_FPS, TrustedWgcPacket, stream_wgc_fixture, verify_wgc_stream,
};
use intralink_transport::{
    bind,
    desktop::{
        Capture, CaptureRequest, H264Frame, bind_h264_diagnostic,
        serve_h264_diagnostic_with_waiter, view_h264_diagnostic,
    },
};
use iroh::SecretKey;
use tokio::sync::{mpsc, oneshot};

use super::precise_timer::TimerWorker;

type DecodeJob = (TrustedWgcPacket, oneshot::Sender<()>);

fn source_worker(
    pid: u32,
    title: &'static str,
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    mut requests: mpsc::Receiver<CaptureRequest>,
    expected: mpsc::Sender<TrustedWgcPacket>,
) -> Result<usize> {
    // Start native capture only after the authenticated transport asks.
    let Some(first) = requests.blocking_recv() else {
        return Ok(0);
    };
    let pending = RefCell::new(Some(first));
    let delivered = stream_wgc_fixture(
        pid,
        title,
        height,
        samples,
        adapter_index,
        || {
            if pending.borrow().is_none() {
                *pending.borrow_mut() = requests.blocking_recv();
            }
            Ok(pending.borrow().is_some())
        },
        |packet| {
            let req = pending
                .borrow_mut()
                .take()
                .context("No outstanding capture demand")?;
            let mut buffer = req.buffer;
            h264::wrap(
                packet.sequence(),
                packet.keyframe(),
                packet.bytes(),
                &mut buffer,
            )
            .map_err(|error| anyhow::anyhow!("Wrap native H.264: {error:?}"))?;
            // Enqueue the immutable native origin token BEFORE publishing
            // its wire copy. This bounded queue cannot exceed wire credit:
            // the viewer consumes the corresponding token before every ACK.
            if expected.blocking_send(packet).is_err() {
                return Ok(false);
            }
            Ok(req.reply.send(Ok(Capture::Frame(buffer))).is_ok())
        },
    )?;
    if delivered == usize::from(samples)
        && let Some(end) = requests.blocking_recv()
    {
        let _ = end.reply.send(Ok(Capture::End));
    }
    Ok(delivered)
}

fn viewer_worker(
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    mut jobs: mpsc::Receiver<DecodeJob>,
) -> Result<usize> {
    verify_wgc_stream(height, samples, adapter_index, || {
        let Some((packet, done)) = jobs.blocking_recv() else {
            return Ok(None);
        };
        Ok(Some((packet, move || Ok(done.send(()).is_ok()))))
    })
}

fn join_worker<T>(worker: JoinHandle<Result<T>>, name: &str) -> Result<T> {
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("{name} worker panicked"))?
        .with_context(|| format!("{name} worker failed"))
}

fn verify_wire(
    profile: Profile,
    frame: H264Frame,
    sequence: u32,
    keyframe: bool,
    origin: &[u8],
) -> Result<()> {
    let received = h264::unwrap(&frame.payload)
        .map_err(|error| anyhow::anyhow!("Invalid H.264 envelope: {error:?}"))?;
    ensure!(
        frame.profile == profile
            && frame.record_sequence == sequence as usize
            && received.sequence == sequence
            && received.keyframe == keyframe
            && received.annex_b == origin,
        "Received H.264 differs from native origin"
    );
    Ok(())
}

pub(super) async fn run(
    pid: u32,
    title: &'static str,
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    cancel_after: Option<u16>,
) -> Result<()> {
    ensure!((1..=180).contains(&samples), "Use 1-180 generated frames");
    ensure!(
        cancel_after.is_none_or(|count| count > 0 && count < samples),
        "Cancellation must precede the final frame"
    );
    let profile = Profile::new(height, FIXTURE_FPS, 30, FIXTURE_BITRATE)
        .context("Invalid live diagnostic profile")?;
    let mut waiter = TimerWorker::start()?;
    tokio::time::timeout(Duration::from_secs(2), waiter.ready())
        .await
        .context("Precise timer startup timed out")??;
    let local: SocketAddr = "127.0.0.1:0".parse()?;
    let host = bind_h264_diagnostic(SecretKey::generate(), local).await?;
    let viewer = match bind(SecretKey::generate(), local, false).await {
        Ok(endpoint) => endpoint,
        Err(error) => {
            host.close().await;
            return Err(error);
        }
    };

    let result = async {
        let addr = host.bound_sockets().into_iter().find(SocketAddr::is_ipv4)
            .context("Missing loopback socket")?;
        let (source, requests) = mpsc::channel::<CaptureRequest>(1);
        let (expected, mut expected_rx) = mpsc::channel::<TrustedWgcPacket>(MAX_IN_FLIGHT_RECORDS);
        let (decode, jobs) = mpsc::channel::<DecodeJob>(1);
        let capture_worker = thread::Builder::new().name("intralink-wgc-source".into())
            .spawn(move || source_worker(pid,title,height,samples,adapter_index,requests,expected))?;
        let decode_worker = match thread::Builder::new().name("intralink-gpu-viewer".into())
            .spawn(move || viewer_worker(height,samples,adapter_index,jobs)) {
            Ok(worker) => worker,
            Err(error) => {
                drop(source);
                drop(expected_rx);
                let _ = join_worker(capture_worker,"capture");
                return Err(error.into());
            }
        };
        let started=Instant::now();
        let canceled=Arc::new(AtomicBool::new(false));
        let stop_flag=canceled.clone();
        // Both channel owners live inside the transport futures. Cancellation
        // drops them before the native worker joins; blocking_recv/send wake.
        let transport = tokio::try_join!(
            serve_h264_diagnostic_with_waiter(&host, viewer.id(), profile, source, &mut waiter),
            view_h264_diagnostic(&viewer, host.id(), std::slice::from_ref(&addr), profile, move |frame| {
                let verified = (|| {
                    let packet = expected_rx.try_recv().context("Missing ordered native origin")?;
                    verify_wire(profile, frame, packet.sequence(), packet.keyframe(), packet.bytes())?;
                    Ok::<_,anyhow::Error>(packet)
                })();
                let decode = decode.clone();
                let stop_flag=stop_flag.clone();
                async move {
                    let packet = verified?;
                    let sequence=packet.sequence();
                    let (done, receipt) = oneshot::channel();
                    decode.send((packet,done)).await
                        .map_err(|_| anyhow::anyhow!("Decode worker stopped"))?;
                    receipt.await.context("Native decoder rejected packet")?;
                    if cancel_after.map(u32::from)==Some(sequence) {
                        stop_flag.store(true,Ordering::Release);
                        anyhow::bail!("Local diagnostic viewer cancellation");
                    }
                    Ok(())
                }
            })
        );
        let elapsed=started.elapsed();
        // Native calls are synchronous; channel cancellation does not interrupt
        // a hung driver call. The executable-level gate enforces a hard budget.
        let capture_result=join_worker(capture_worker,"capture");
        let decoder_result=join_worker(decode_worker,"decoder");
        if let Err(error)=&capture_result {eprintln!("{error:#}");}
        if let Err(error)=&decoder_result {eprintln!("{error:#}");}
        let delivered=capture_result?;
        let submitted=decoder_result?;
        if let Some(stop)=cancel_after {
            ensure!(canceled.load(Ordering::Acquire) && transport.is_err()
                && submitted==usize::from(stop)
                && delivered>=submitted
                && delivered<=submitted+MAX_IN_FLIGHT_RECORDS,
                "Cancellation did not stop ordered GPU workers within credit");
            println!("wgc_live_cancel=passed, requested={stop}, source_delivered={delivered}, viewer_submitted={submitted}, joined_workers=true, no_more_decode=true");
            return Ok(());
        }
        let (served,viewed)=transport?;
        ensure!(delivered==usize::from(samples) && submitted==delivered,"Native worker frame counts mismatch");
        ensure!(served==viewed && served.frames==usize::from(samples)
            && served.repeats==0 && served.no_updates==0,"Live diagnostic frame counts mismatch");
        println!("wgc_live_loopback=passed, resolution={}x{}, frames={}, bytes={}, elapsed_ms={:.3}, wire_pipeline_fps={:.2}, authenticated_direct_quic=true, exact_native_origin=true, independent_gpu_workers=true, joined_workers=true, credit_limit={}, decode_queue=1",
            profile.width(),profile.height(),served.frames,served.media_bytes,elapsed.as_secs_f64()*1000.0,
            f64::from(samples)/elapsed.as_secs_f64(),MAX_IN_FLIGHT_RECORDS);
        Ok::<(),anyhow::Error>(())
    }.await;
    viewer.close().await;
    host.close().await;
    waiter.close()?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_native_origin_is_required_before_decode() -> Result<()> {
        let profile = Profile::new(540, 60, 30, FIXTURE_BITRATE).unwrap();
        // Structural envelope fixture only; never submitted to a driver.
        let bytes = [0, 0, 1, 0x65, 0xb8];
        let make = |profile, record_sequence, sequence, keyframe, data: &[u8]| {
            let mut payload = Vec::new();
            h264::wrap(sequence, keyframe, data, &mut payload).unwrap();
            H264Frame {
                profile,
                record_sequence,
                payload,
            }
        };
        verify_wire(profile, make(profile, 1, 1, true, &bytes), 1, true, &bytes)?;
        assert!(verify_wire(profile, make(profile, 2, 1, true, &bytes), 1, true, &bytes).is_err());
        assert!(verify_wire(profile, make(profile, 1, 2, true, &bytes), 1, true, &bytes).is_err());
        assert!(verify_wire(profile, make(profile, 1, 1, false, &bytes), 1, true, &bytes).is_err());
        let mut changed = bytes;
        changed[4] ^= 1;
        assert!(
            verify_wire(
                profile,
                make(profile, 1, 1, true, &changed),
                1,
                true,
                &bytes
            )
            .is_err()
        );
        let other = Profile::new(720, 60, 30, FIXTURE_BITRATE).unwrap();
        assert!(verify_wire(profile, make(other, 1, 1, true, &bytes), 1, true, &bytes).is_err());
        Ok(())
    }
}
