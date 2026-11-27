//! Local acceptance viewer for a disposable AVD or owner-selected Android.
//! No recording, user vault or remote input. Generated Surface mode checks
//! every pixel; approved-preview mode requires fresh on-device consent.
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
    let selection = args.next();
    let physical = selection.as_deref() == Some("--surface-fixture");
    let navigation = selection.as_deref() == Some("--approved-navigation");
    let preview = selection.as_deref() == Some("--approved-preview") || navigation;
    ensure!(
        selection.is_none() || physical || preview,
        "Unknown acceptance mode"
    );
    let allowed = if physical || preview {
        matches!(remote, SocketAddr::V4(v) if v.ip().is_private())
    } else {
        remote.ip().is_loopback()
    };
    ensure!(
        allowed && remote.port() >= 1024 && args.next().is_none(),
        "Require explicit local emulator or private approved Android target"
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
    let mode = ViewMode::new(CodecMode::Compact, ViewProtocol::Bounded).with_limits(
        ReceiverLimits::new(
            if physical || preview { 1080 } else { 720 },
            if preview { 60 } else { 30 },
        )
        .unwrap(),
    );
    let mut window = Window::new(
        if physical {
            "IntraLink - physical generated Surface acceptance (ESC to stop)"
        } else if navigation {
            "IntraLink - owner-approved entire-display navigation (ESC to stop)"
        } else if preview {
            "IntraLink - owner-approved Android screen preview (ESC to stop)"
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
            Duration::from_secs(if navigation { 110 } else { 30 }),
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
    let mut withheld = 0usize;
    let mut visible = !preview;
    let mut blank = Vec::new();
    let mut first_decoded: Option<Instant> = None;
    let mut last_marker_counts = (0usize, 0usize, 0usize);
    let mut bytes = 0u64;
    let mut jpeg_frames = 0usize;
    let mut tile_frames = 0usize;
    let mut lossless_frames = 0usize;
    let mut last_sample = Instant::now();
    let result = (|| -> Result<()> {
        while window.is_open()
            && !window.is_key_down(Key::Escape)
            && started.elapsed() < Duration::from_secs(if navigation { 115 } else { 35 })
        {
            let mut new_visible = false;
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
                bytes += frame.payload.len() as u64;
                match frame.payload.first().copied() {
                    Some(intralink_core::media::JPEG_TAG) => jpeg_frames += 1,
                    Some(intralink_core::media::TILE_TAG) => tile_frames += 1,
                    Some(intralink_core::media::LOSSLESS_KEYFRAME_TAG) => lossless_frames += 1,
                    _ => anyhow::bail!("Unexpected media tag"),
                }
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
                let mut black = 0usize;
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
                    if near(0, 0, 0) {
                        black += 1;
                    }
                    hash = (hash ^ u64::from(pixel)).wrapping_mul(0x100000001b3);
                }
                let recognized = navy > 100 && mint > 10;
                if !physical && recognized {
                    fixture_frames += 1;
                }
                last_marker_counts = (navy, mint, black);
                first_decoded.get_or_insert_with(Instant::now);
                // The approved surface may first publish a blank transition.
                // Continue ordered decoding, but never paint an unrecognized
                // screen. Once the fixture is visible, loss ends the test.
                if preview && !recognized && (!navigation || !visible) {
                    if visible {
                        anyhow::bail!("Approved preview fixture disappeared");
                    }
                    withheld += 1;
                    blank.resize(pixels.len(), 0);
                } else {
                    visible = true;
                    new_visible = true;
                    if previous != Some(hash) {
                        changed += 1;
                    }
                    previous = Some(hash);
                    first_frame.get_or_insert(started.elapsed().as_millis());
                    last_frame = Some(started.elapsed().as_millis());
                }
                seq = frame.sequence;
                frames += 1;
            }
            if preview
                && !visible
                && first_decoded.is_some_and(|t| t.elapsed() >= Duration::from_secs(3))
            {
                anyhow::bail!(
                    "Preview startup not visible: received={frames}, withheld={withheld}, navy/mint/black={last_marker_counts:?}"
                );
            }
            if !visible {
                blank.resize(dimensions.0 * dimensions.1, 0);
            }
            let painting = Instant::now();
            window
                .update_with_buffer(
                    if visible { &pixels } else { &blank },
                    dimensions.0,
                    dimensions.1,
                )
                .map_err(|e| anyhow::anyhow!("Presentation failed: {e}"))?;
            if new_visible {
                presented += 1;
                presentation_us += painting.elapsed().as_micros();
                if navigation && presented == 1 {
                    println!("NAVIGATION_READY");
                    io::stdout().flush()?;
                }
            }
            if navigation && last_sample.elapsed() >= Duration::from_secs(1) {
                println!(
                    "NAVIGATION_SAMPLE ms={} frames={} presented={} bytes={} jpeg={} tiles={} lossless={} decode_us={}",
                    started.elapsed().as_millis(),
                    frames,
                    presented,
                    bytes,
                    jpeg_frames,
                    tile_frames,
                    lossless_frames,
                    decode_us
                );
                io::stdout().flush()?;
                last_sample = Instant::now();
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
    ensure!(bytes == stats.media_bytes, "Media byte count mismatch");
    ensure!(
        presented + withheld == frames && frames == stats.frames,
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
            presented >= 5 && changed >= 5 && fixture_frames >= 3,
            "Insufficient changing fixture frames: decoded={frames}, changed={changed}, fixture={fixture_frames}"
        );
    }
    if navigation {
        println!(
            "NAVIGATION_FINAL jpeg={} tiles={} lossless={}",
            jpeg_frames, tile_frames, lossless_frames
        );
    }
    println!(
        "EMULATOR_VIEW_PASSED frames={} bytes={} repeats={} no_updates={} changed={} fixture={} first_ms={} total_ms={} decode_us={} presented={} active_span_ms={} presentation_us={} withheld={}",
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
        presentation_us,
        withheld
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
