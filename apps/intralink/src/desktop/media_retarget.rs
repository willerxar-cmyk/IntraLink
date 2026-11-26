//! Native output-retarget diagnostic for one generated local WGC window.
use super::super::codec::{Decoder, Encoder};
use super::{Fixture, TITLE, find_fixture, verify_pattern};
use anyhow::{Context, Result, ensure};
use intralink_core::desktop::{CodecMode, VideoConfig};
use intralink_transport::desktop::Capture;
use std::{
    process::Command,
    time::{Duration, Instant},
};

/// Exercise one owned capture session through downshift and recovery. This
/// is a local fixture gate, not a physical desktop or network benchmark.
#[cfg(feature = "wgc-preview")]
pub fn retarget_check() -> Result<()> {
    use intralink_core::adaptive::{ViewProtocol, config_at_height};
    let ceiling = VideoConfig::new(1080, 60, 75, 30)
        .context("Invalid fixture profile")?
        .with_mode(CodecMode::Compact);
    let mut child = Fixture(
        Command::new(std::env::current_exe()?)
            .args(["desktop-test-pattern", "--height", "1080"])
            .spawn()
            .context("Start owned retarget fixture")?,
    );
    let _target = find_fixture(&mut child.0)?;
    let mut session = super::super::wgc_check::CaptureSession::start_fixture(
        child.0.id(),
        TITLE,
        Some((1920, 1080)),
    )?;
    assert!(session.retarget((2560, 1440)).is_err());
    let mut encoder = Encoder::new(ceiling);
    let mut decoder = Decoder::default();
    let mut payload = Vec::new();
    let mut pixels = Vec::new();
    let mut native = None;
    let mut prior_bytes = 0;
    let started = Instant::now();
    for (index, height) in [1080, 720, 540, 720, 1080].into_iter().enumerate() {
        let config =
            config_at_height(ceiling, height).context("Invalid retarget fixture height")?;
        let target = (config.width(), config.height());
        session.retarget(target)?;
        encoder.reconfigure(config)?;
        let mut observed = 0u64;
        let mut tier_readback = 0u64;
        let mut keyframe = false;
        while observed < 6 {
            ensure!(
                started.elapsed() < Duration::from_secs(25),
                "Retarget fixture deadline exceeded"
            );
            let Some((frame, native_size)) = session.capture(|| {
                ensure!(
                    child.0.try_wait()?.is_none(),
                    "Owned retarget fixture exited"
                );
                Ok(())
            })?
            else {
                continue;
            };
            ensure!(
                native.is_none_or(|size| size == native_size),
                "Native source changed during retarget"
            );
            native = Some(native_size);
            let expected = super::super::resize::fitted(native_size, target);
            ensure!(
                frame.dimensions() == expected,
                "Incorrect native scaler output"
            );
            verify_pattern(&frame, target.0 as usize, target.1 as usize)?;
            tier_readback += frame.as_raw().len() as u64;
            let record = encoder.encode(&frame, payload)?;
            session.recycle(frame);
            match record {
                Capture::Frame(data) => {
                    if observed == 0 {
                        ensure!(
                            intralink_core::adaptive::is_keyframe(config.mode(), &data),
                            "Transition without keyframe"
                        );
                        keyframe = true;
                    }
                    decoder.decode_into_protocol(
                        config,
                        &data,
                        &mut pixels,
                        ViewProtocol::Adaptive,
                    )?;
                    payload = data;
                }
                Capture::Repeat(data) => {
                    ensure!(observed > 0, "Repeat before retarget keyframe");
                    payload = data;
                }
                _ => anyhow::bail!("Unexpected local retarget record"),
            }
            ensure!(
                pixels.len() == (target.0 * target.1) as usize,
                "Wrong decoded canvas"
            );
            observed += 1;
        }
        let stats = session.stats()?;
        ensure!(
            stats.gpu_processors == index as u64 + 1
                && stats.staging_allocations == index as u64 + 1
                && stats.readback_frames == (index as u64 + 1) * 6
                && stats.readback_bytes == prior_bytes + tier_readback
                && keyframe,
            "Unexpected retarget resource lifetime or readback counters"
        );
        prior_bytes = stats.readback_bytes;
        println!(
            "retarget_height={height}, native={:?}, scaled={:?}, samples={observed}, readback_bytes={tier_readback}, processors={}, staging={}",
            native_size_for_log(native)?,
            super::super::resize::fitted(native_size_for_log(native)?, target),
            stats.gpu_processors,
            stats.staging_allocations
        );
    }
    let stats = session.stats()?;
    session.stop()?;
    ensure!(
        session.retarget((960, 540)).is_err(),
        "Retarget accepted after stop"
    );
    println!(
        "wgc_retarget_check=passed, samples={}, processors={}, staging={}, rgba_allocations={}, total_readback_bytes={}",
        stats.readback_frames,
        stats.gpu_processors,
        stats.staging_allocations,
        stats.rgba_allocations,
        stats.readback_bytes
    );
    Ok(())
}

#[cfg(feature = "wgc-preview")]
fn native_size_for_log(native: Option<(u32, u32)>) -> Result<(u32, u32)> {
    native.context("Missing native fixture geometry")
}
