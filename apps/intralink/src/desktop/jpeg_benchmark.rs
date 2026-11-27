//! Local A/B diagnostic, using only generated pixels. The Image JPEG encoder
//! is the historical reference; normal streaming calls only jpeg::encode_rgb.
use anyhow::{Context, Result, ensure};
use image::{ExtendedColorType, codecs::jpeg::JpegEncoder};
use intralink_core::{
    desktop::{MAX_FRAME_BYTES, VideoConfig},
    media::JPEG_TAG,
};
use std::time::{Duration, Instant};

use super::{codec::Decoder, jpeg::encode_rgb, resize::CanvasResizer};

const SAMPLES: usize = 12;

fn reference(config: VideoConfig, rgb: &[u8], bytes: &mut Vec<u8>) -> Result<()> {
    bytes.clear();
    bytes.push(JPEG_TAG);
    JpegEncoder::new_with_quality(&mut *bytes, config.quality()).encode(
        rgb,
        config.width(),
        config.height(),
        ExtendedColorType::Rgb8,
    )?;
    ensure!(
        bytes.len() <= MAX_FRAME_BYTES,
        "Reference diagnostic fixture exceeds frame limit"
    );
    Ok(())
}

fn current(config: VideoConfig, rgb: &[u8], bytes: &mut Vec<u8>) -> Result<()> {
    encode_rgb(config, config.quality(), rgb, bytes)
}

fn timed(
    encode: impl FnOnce(VideoConfig, &[u8], &mut Vec<u8>) -> Result<()>,
    config: VideoConfig,
    rgb: &[u8],
    bytes: &mut Vec<u8>,
) -> Result<Duration> {
    let started = Instant::now();
    encode(config, rgb, bytes)?;
    let elapsed = started.elapsed();
    ensure!(!elapsed.is_zero(), "Local clock cannot measure encoder");
    Ok(elapsed)
}

fn metrics(label: &str, height: u16, bytes: usize, mut timings: Vec<Duration>) {
    timings.sort_unstable();
    let ms = |p: usize| timings[(timings.len() * p).div_ceil(100) - 1].as_secs_f64() * 1000.0;
    println!(
        "encoder={label}, height={height}, quality=75, sampling=444, samples={}, bytes={bytes}, p50_ms={:.3}, p95_ms={:.3}, worst_ms={:.3}",
        timings.len(),
        ms(50),
        ms(95),
        ms(100)
    );
}

pub(super) fn run() -> Result<()> {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    let avx2 = cfg!(feature = "jpeg-simd") && std::is_x86_feature_detected!("avx2");
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let avx2 = false;
    println!(
        "Local JPEG encode comparison: simd_feature={}, runtime_avx2={avx2}",
        cfg!(feature = "jpeg-simd")
    );
    for height in [1080, 720, 540] {
        let config =
            VideoConfig::new(height, 60, 75, 30).context("Invalid encoder check profile")?;
        let size = (config.width(), config.height());
        let frame = super::host_calibration::dense_fixture(size)?;
        let mut rgb = vec![0; (config.width() * config.height() * 3) as usize];
        CanvasResizer::default().fill_rgb(&frame, &mut rgb, size)?;
        let mut old = Vec::new();
        let mut new = Vec::new();
        reference(config, &rgb, &mut old)?;
        current(config, &rgb, &mut new)?;
        let mut old_times = Vec::with_capacity(SAMPLES);
        let mut new_times = Vec::with_capacity(SAMPLES);
        for sample in 0..SAMPLES {
            // Alternate order to avoid giving one implementation all the
            // cold/warm scheduling and clock effects.
            if sample.is_multiple_of(2) {
                old_times.push(timed(reference, config, &rgb, &mut old)?);
                new_times.push(timed(current, config, &rgb, &mut new)?);
            } else {
                new_times.push(timed(current, config, &rgb, &mut new)?);
                old_times.push(timed(reference, config, &rgb, &mut old)?);
            }
        }
        // Verify both payloads with the actual production receiver, not a
        // benchmark-only decoder. The fixture keeps a central color marker.
        for payload in [&old, &new] {
            let mut pixels = Vec::new();
            Decoder::default().decode_into(config, payload, &mut pixels)?;
            super::host_calibration::verify_marker(config, &pixels)?;
        }
        metrics("image_reference", height, old.len(), old_times);
        metrics("bounded_rust_jpeg", height, new.len(), new_times);
    }
    println!("encoder_check=passed; local generated pixels only, no capture or network.");
    Ok(())
}
