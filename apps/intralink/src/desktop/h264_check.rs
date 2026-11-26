//! Isolated H.264 codec/QUIC diagnostic. Never opens a vault, real screen,
//! microphone, remote address or production sharing session.
use std::{
    future::ready,
    net::SocketAddr,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use intralink_core::h264::{self, Profile};
use intralink_d3d11::{EncodedFixture, FIXTURE_BITRATE, FIXTURE_FPS};
use intralink_transport::{
    bind,
    desktop::{
        Capture, CaptureRequest, PacerWait, bind_h264_diagnostic,
        serve_h264_diagnostic_with_waiter, view_h264_diagnostic,
    },
};
use iroh::SecretKey;
use tokio::sync::mpsc;

use super::precise_timer::TimerWorker;

enum DiagnosticWait {
    Standard,
    Precise(TimerWorker),
}

impl PacerWait for DiagnosticWait {
    async fn wait(&mut self, delay: Duration) -> Result<()> {
        match self {
            Self::Standard => {
                tokio::time::sleep(delay).await;
                Ok(())
            }
            Self::Precise(timer) => timer.wait(delay).await,
        }
    }
}

/// Use the previously verified native encoder and the actual QUIC transport.
/// Encoding is completed before transfer; decoding starts after it. The
/// receiver checks every byte against the generated fixture before native
/// decode, so this is neither a live desktop path nor a hostile-bitstream test.
pub async fn run(
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    precise_timer: bool,
) -> Result<()> {
    ensure!((1..=180).contains(&samples), "Use 1-180 generated frames");
    let profile = Profile::new(height, FIXTURE_FPS, 30, FIXTURE_BITRATE)
        .context("Unsupported H.264 diagnostic profile")?;
    let fixture = EncodedFixture::generate(height, samples, adapter_index)?;
    ensure!(
        fixture.len() == usize::from(samples),
        "Generated frame count mismatch"
    );
    let mut waiter = if precise_timer {
        let mut timer = TimerWorker::start()?;
        tokio::time::timeout(Duration::from_secs(2), timer.ready())
            .await
            .context("Precise timer startup timed out")??;
        DiagnosticWait::Precise(timer)
    } else {
        DiagnosticWait::Standard
    };
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
            .context("Missing diagnostic loopback socket")?;
        let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
        let producer = async {
            for i in 0..fixture.len() {
                let req = requests.recv().await.context("Missing H.264 capture demand")?;
                let (key, data) = fixture.packet(i).context("Missing fixture access unit")?;
                let mut buffer = req.buffer;
                h264::wrap((i + 1) as u32, key, data, &mut buffer)
                    .map_err(|error| anyhow::anyhow!("Wrap H.264 fixture: {error:?}"))?;
                ensure!(req.reply.send(Ok(Capture::Frame(buffer))).is_ok(), "H.264 fixture canceled");
            }
            let req = requests.recv().await.context("Missing end demand")?;
            ensure!(req.reply.send(Ok(Capture::End)).is_ok(), "H.264 fixture canceled at end");
            Ok::<(), anyhow::Error>(())
        };
        let mut received = Vec::with_capacity(fixture.len());
        let mut verified_bytes = 0usize;
        let started = Instant::now();
        let (served, viewed, ()) = tokio::try_join!(
            serve_h264_diagnostic_with_waiter(&host, viewer.id(), profile, source, &mut waiter),
            view_h264_diagnostic(&viewer, host.id(), std::slice::from_ref(&addr), profile, |frame| {
                let result = (|| {
                    let index = received.len();
                    let (key, expected) = fixture.packet(index).context("Unexpected extra H.264 frame")?;
                    let access = h264::unwrap(&frame.payload)
                        .map_err(|error| anyhow::anyhow!("Unwrap H.264 fixture: {error:?}"))?;
                    ensure!(frame.profile == profile && frame.record_sequence == index + 1
                        && access.sequence as usize == index + 1 && access.keyframe == key
                        && access.annex_b == expected, "H.264 diagnostic frame mismatch");
                    verified_bytes = verified_bytes.checked_add(frame.payload.len())
                        .context("H.264 diagnostic byte count overflow")?;
                    // The exact expected packet is checked before retaining it.
                    received.push(frame.payload);
                    Ok(())
                })();
                ready(result)
            }),
            producer
        )?;
        let elapsed = started.elapsed();
        ensure!(served == viewed && served.frames == fixture.len() && served.repeats == 0
            && served.no_updates == 0 && served.media_bytes == verified_bytes as u64,
            "H.264 diagnostic receipt or media count mismatch");
        fixture.verify_received(received.iter().map(|packet| &packet[h264::AU_HEADER..]))?;
        println!("h264_loopback_check=passed, resolution={}x{}, frames={}, bytes={}, elapsed_ms={:.3}, wire_rate_fps={:.2}, profile_fps={}, precise_timer={precise_timer}, authenticated_direct_quic=true, exact_fixture_match=true",
            profile.width(), profile.height(), received.len(), verified_bytes,
            elapsed.as_secs_f64()*1000.0, received.len() as f64/elapsed.as_secs_f64(), profile.fps());
        println!("Pre-encoded synthetic loopback transfer followed by native decode; not live capture or network presentation.");
        Ok::<(), anyhow::Error>(())
    }.await;
    viewer.close().await;
    host.close().await;
    let cleanup = match &mut waiter {
        DiagnosticWait::Standard => Ok(()),
        DiagnosticWait::Precise(timer) => timer.close(),
    };
    cleanup?;
    result
}
