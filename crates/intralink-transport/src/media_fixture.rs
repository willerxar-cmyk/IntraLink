//! Explicit generated media diagnostic over the existing v12 transport.
//! No OS capture, arbitrary native decoder, input or persistent authorization.
use crate::desktop::{
    Capture, CaptureRequest, Stats, ViewMode, serve_view_with_protocol, view_with_observations,
};
use anyhow::{Context, Result, ensure};
use intralink_core::{
    adaptive::ViewProtocol,
    desktop::{CodecMode, VideoConfig, ViewPriority},
    media::{Delta, MAX_DELTA_BYTES, apply_delta, apply_keyframe, encode_delta, encode_keyframe},
    profile_limits::ReceiverLimits,
};
use iroh::{Endpoint, EndpointId};
use std::{future::ready, net::SocketAddr, time::Duration};
use tokio::{sync::mpsc, time::timeout};

pub const FRAMES: usize = 60;
pub const HEIGHT: u16 = 1080;
pub const FPS: u32 = 30;
const WIDTH: usize = 1920;
const PIXELS: usize = WIDTH * HEIGHT as usize;
const WAIT: Duration = Duration::from_secs(5);
pub const PROTOCOL: ViewProtocol = ViewProtocol::Bounded;

pub fn config() -> VideoConfig {
    VideoConfig::new(HEIGHT, FPS, 75, 20)
        .expect("Fixed fixture config")
        .with_mode(CodecMode::Compact)
}

fn color(x: usize, y: usize, frame: usize) -> [u8; 3] {
    // Distinct color bands, moving 64x64 marker. The independently checked
    // expected pixels include every pixel, not merely selected samples.
    let marker = (frame * 19) % (WIDTH - 64);
    if (marker..marker + 64).contains(&x) && (128..192).contains(&y) {
        [250, 219, 35]
    } else {
        match (x * 6 / WIDTH) % 6 {
            0 => [16, 16, 16],
            1 => [240, 240, 240],
            2 => [220, 24, 18],
            3 => [15, 200, 40],
            4 => [28, 38, 228],
            _ => [45, 169, 179],
        }
    }
}

fn render(frame: usize, rgb: &mut Vec<u8>) {
    rgb.resize(PIXELS * 3, 0);
    for (index, pixel) in rgb.chunks_exact_mut(3).enumerate() {
        pixel.copy_from_slice(&color(index % WIDTH, index / WIDTH, frame));
    }
}

/// Test-only generated source, explicit one-peer endpoint supplied by caller.
pub async fn serve(endpoint: &Endpoint, approved: EndpointId) -> Result<Stats> {
    let config = config();
    let (source, mut requests) = mpsc::channel::<CaptureRequest>(1);
    let producer = async {
        let mut previous = Vec::new();
        let mut current = Vec::new();
        for frame in 0..FRAMES {
            let req = timeout(WAIT, requests.recv())
                .await?
                .context("No fixture capture demand")?;
            ensure!(
                req.desktop_config == Some(config),
                "Fixture profile changed"
            );
            render(frame, &mut current);
            let mut payload = req.buffer;
            if frame == 0 {
                ensure!(
                    encode_keyframe(config, &current, &mut payload, MAX_DELTA_BYTES)
                        .map_err(|e| anyhow::anyhow!("{e:?}"))?,
                    "Fixture keyframe exceeded budget"
                );
            } else {
                ensure!(
                    encode_delta(config, &previous, &current, &mut payload, MAX_DELTA_BYTES)
                        .map_err(|e| anyhow::anyhow!("{e:?}"))?
                        == Delta::Encoded,
                    "Fixture delta not encoded"
                );
            }
            std::mem::swap(&mut previous, &mut current);
            ensure!(
                req.reply
                    .send(Ok(Capture::AdaptiveFrame {
                        height: HEIGHT,
                        data: payload
                    }))
                    .is_ok(),
                "Fixture capture canceled"
            );
        }
        let req = timeout(WAIT, requests.recv())
            .await?
            .context("No final fixture demand")?;
        ensure!(
            req.reply.send(Ok(Capture::End)).is_ok(),
            "Fixture end canceled"
        );
        Ok::<(), anyhow::Error>(())
    };
    let (stats, ()) = tokio::try_join!(
        serve_view_with_protocol(endpoint, approved, config, source, PROTOCOL),
        producer
    )?;
    ensure!(
        stats.frames == FRAMES && stats.repeats == 0 && stats.no_updates == 0,
        "Wrong fixture send count"
    );
    Ok(stats)
}

/// Reconstruct and compare every RGB pixel on the real receiver. No native
/// decoder or display path is used. Reject a different negotiated profile.
pub async fn view(endpoint: &Endpoint, expected: EndpointId, remote: SocketAddr) -> Result<Stats> {
    let config = config();
    let mode = ViewMode::new(CodecMode::Compact, PROTOCOL)
        .with_limits(ReceiverLimits::new(HEIGHT, FPS).unwrap());
    let mut canvas = Vec::new();
    let mut frames = 0usize;
    let stats = view_with_observations(
        endpoint,
        expected,
        &[remote],
        mode,
        Vec::new,
        |frame| {
            let result = (|| -> Result<()> {
                ensure!(
                    frame.config == config && frame.sequence == frames + 1 && frames < FRAMES,
                    "Unexpected fixture profile or sequence"
                );
                if frames == 0 {
                    apply_keyframe(config, &frame.payload, &mut canvas)
                        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
                } else {
                    apply_delta(config, &frame.payload, &mut canvas)
                        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
                }
                ensure!(canvas.len() == PIXELS, "Wrong reconstructed pixel count");
                for (index, actual) in canvas.iter().enumerate() {
                    let [r, g, b] = color(index % WIDTH, index / WIDTH, frames);
                    let expected = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
                    ensure!(
                        *actual == expected,
                        "Wrong pixel at frame {frames}, index {index}"
                    );
                }
                frames += 1;
                Ok(())
            })();
            ready(result)
        },
        |_| ViewPriority::Foreground,
    )
    .await?;
    ensure!(
        frames == FRAMES && stats.frames == FRAMES && stats.repeats == 0 && stats.no_updates == 0,
        "Incomplete fixture receipt"
    );
    Ok(stats)
}
