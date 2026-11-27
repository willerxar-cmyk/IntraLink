//! Local acceptance viewer for the disposable AVD, or an explicitly selected
//! physical Android generated-Surface fixture. No recording, user vault or
//! remote input. Physical mode validates every generated pixel before painting.
#[path = "../src/desktop/decoder.rs"]
mod decoder;

use anyhow::{Context, Result, ensure};
use intralink_core::{
    adaptive::ViewProtocol,
    desktop::{CodecMode, ViewPriority},
    profile_limits::ReceiverLimits,
};
use intralink_transport::{
    bind_controller,
    desktop::{VideoFrame, ViewMode, view_with_observations},
};
use iroh::{EndpointId, SecretKey};
use minifb::{Key, ScaleMode, Window, WindowOptions};
use std::{
    io::{self, Write},
    net::SocketAddr,
    time::{Duration, Instant},
};
use tokio::{sync::mpsc, time::timeout};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let remote: SocketAddr = args
        .next()
        .context("Supply loopback UDP forward")?
        .parse()?;
    let physical = match args.next().as_deref() {
        None => false,
        Some("--surface-fixture") => true,
        _ => anyhow::bail!("Unknown acceptance mode"),
    };
    let allowed = if physical {
        matches!(remote, SocketAddr::V4(v) if v.ip().is_private())
    } else {
        remote.ip().is_loopback()
    };
    ensure!(
        allowed && remote.port() >= 1024 && args.next().is_none(),
        "Require explicit local emulator or private generated-Surface target"
    );
    let endpoint = bind_controller(SecretKey::generate(), &[remote]).await?;
    println!("EMULATOR_BASE_ID {}", endpoint.id());
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let peer: EndpointId = line.trim().parse()?;
    ensure!(
        peer.to_string() == line.trim() && peer != endpoint.id(),
        "Invalid peer identity"
    );
    let mode = ViewMode::new(CodecMode::Compact, ViewProtocol::Bounded)
        .with_limits(ReceiverLimits::new(if physical { 1080 } else { 720 }, 30).unwrap());
    let mut window = Window::new(
        if physical {
            "IntraLink - physical generated Surface acceptance (ESC to stop)"
        } else {
            "IntraLink - dedicated Android preview acceptance (ESC to stop)"
        },
        960,
        540,
        WindowOptions {
            resize: true,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| anyhow::anyhow!("Cannot create acceptance viewer: {e}"))?;
    window.set_target_fps(60);
    let (tx, mut rx) = mpsc::channel::<VideoFrame>(1);
    let ep = endpoint.clone();
    let task = tokio::spawn(async move {
        timeout(
            Duration::from_secs(30),
            view_with_observations(
                &ep,
                peer,
                &[remote],
                mode,
                Vec::new,
                |frame| {
                    let tx = tx.clone();
                    async move { tx.send(frame).await.context("Viewer closed") }
                },
                |_| ViewPriority::Foreground,
            ),
        )
        .await?
    });
    let started = Instant::now();
    let mut decode = decoder::Decoder::default();
    let mut pixels = vec![0; 960 * 540];
    let mut dimensions = (960, 540);
    let mut frames = 0usize;
    let mut seq = 0;
    let mut changed = 0usize;
    let mut previous = None;
    let mut fixture_frames = 0usize;
    let mut decode_us = 0u128;
    let mut first_frame = None;
    let mut last_frame = None;
    let mut presented = 0usize;
    let mut presentation_us = 0u128;
    let result = (|| -> Result<()> {
        while window.is_open()
            && !window.is_key_down(Key::Escape)
            && started.elapsed() < Duration::from_secs(35)
        {
            if let Ok(frame) = rx.try_recv() {
                ensure!(frame.sequence > seq, "Out-of-order frame");
                let time = Instant::now();
                if mode.protocol == ViewProtocol::Fixed {
                    decode.decode_into(frame.config, &frame.payload, &mut pixels)?;
                } else {
                    decode.decode_into_protocol(
                        frame.config,
                        &frame.payload,
                        &mut pixels,
                        mode.protocol,
                    )?;
                }
                decode_us += time.elapsed().as_micros();
                dimensions = (
                    frame.config.width() as usize,
                    frame.config.height() as usize,
                );
                ensure!(
                    pixels.len() == dimensions.0 * dimensions.1,
                    "Invalid decoded geometry"
                );
                if physical {
                    verify_generated_surface(frames, dimensions, &pixels)?;
                    fixture_frames += 1;
                }
                // Known preview fixture: navy background and moving mint circle.
                // Approximate colors also allow the bounded JPEG fallback.
                let mut navy = 0usize;
                let mut mint = 0usize;
                let mut hash = 0xcbf29ce484222325u64;
                for &pixel in pixels.iter().step_by(17) {
                    let near = |r: i32, g: i32, b: i32| {
                        let a = ((pixel >> 16) & 255) as i32;
                        let c = ((pixel >> 8) & 255) as i32;
                        let d = (pixel & 255) as i32;
                        (a - r).abs() <= 16 && (c - g).abs() <= 16 && (d - b).abs() <= 16
                    };
                    if near(20, 36, 54) {
                        navy += 1;
                    }
                    if near(104, 216, 191) {
                        mint += 1;
                    }
                    hash = (hash ^ u64::from(pixel)).wrapping_mul(0x100000001b3);
                }
                if !physical && navy > 100 && mint > 10 {
                    fixture_frames += 1;
                }
                if previous != Some(hash) {
                    changed += 1;
                }
                previous = Some(hash);
                first_frame.get_or_insert(started.elapsed().as_millis());
                last_frame = Some(started.elapsed().as_millis());
                seq = frame.sequence;
                frames += 1;
            }
            let painting = Instant::now();
            window
                .update_with_buffer(&pixels, dimensions.0, dimensions.1)
                .map_err(|e| anyhow::anyhow!("Presentation failed: {e}"))?;
            if presented < frames {
                presented = frames;
                presentation_us += painting.elapsed().as_micros();
            }
            if task.is_finished() && rx.is_empty() {
                return Ok(());
            }
        }
        anyhow::bail!("Acceptance viewer stopped or timed out")
    })();
    if result.is_err() {
        task.abort();
    }
    let network = task.await;
    endpoint.close().await;
    result?;
    let stats = network??;
    ensure!(
        presented == frames && frames == stats.frames,
        "Presentation/transport count mismatch"
    );
    if physical {
        ensure!(
            frames == 60
                && fixture_frames == 60
                && changed == 60
                && stats.repeats == 0
                && stats.no_updates == 0,
            "Incomplete physical generated-Surface presentation"
        );
    } else {
        ensure!(
            frames >= 5 && changed >= 5 && fixture_frames >= 3,
            "Insufficient changing fixture frames: decoded={frames}, changed={changed}, fixture={fixture_frames}"
        );
    }
    println!(
        "EMULATOR_VIEW_PASSED frames={} bytes={} repeats={} no_updates={} changed={} fixture={} first_ms={} total_ms={} decode_us={} presented={} active_span_ms={} presentation_us={}",
        frames,
        stats.media_bytes,
        stats.repeats,
        stats.no_updates,
        changed,
        fixture_frames,
        first_frame.unwrap_or_default(),
        started.elapsed().as_millis(),
        decode_us,
        presented,
        last_frame
            .unwrap_or_default()
            .saturating_sub(first_frame.unwrap_or_default()),
        presentation_us
    );
    Ok(())
}

fn verify_generated_surface(
    frame: usize,
    dimensions: (usize, usize),
    pixels: &[u32],
) -> Result<()> {
    ensure!(
        frame < 60 && dimensions == (1920, 1080),
        "Wrong generated Surface geometry"
    );
    const BANDS: [u32; 6] = [0x101010, 0xf0f0f0, 0xdc1812, 0x0fc828, 0x1c26e4, 0x2da9b3];
    let marker = frame * 19 % (1920 - 64);
    for (i, &pixel) in pixels.iter().enumerate() {
        let (x, y) = (i % 1920, i / 1920);
        let expected = if (marker..marker + 64).contains(&x) && (128..192).contains(&y) {
            0xfadb23
        } else {
            BANDS[x * 6 / 1920]
        };
        ensure!(
            pixel == expected,
            "Not the generated test fixture (frame {frame}, pixel {i})"
        );
    }
    Ok(())
}
