//! Local A/B diagnostic, using only generated pixels. The Image JPEG encoder
//! is the historical reference; also compare ordered local restart strips.
use anyhow::{Context, Result, ensure};
use image::{ExtendedColorType, codecs::jpeg::JpegEncoder};
use intralink_core::{
    desktop::{MAX_FRAME_BYTES, VideoConfig},
    media::JPEG_TAG,
};
use std::time::{Duration, Instant};

use super::{
    codec::{Decoder, Encoder},
    jpeg::encode_rgb,
    jpeg_parallel::JpegWorkspace,
    resize::CanvasResizer,
};
use intralink_core::desktop::CodecMode;
use intralink_transport::desktop::Capture;

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
        let mut two = JpegWorkspace::with_workers(2);
        let mut four = JpegWorkspace::with_workers(4);
        let mut two_bytes = Vec::new();
        let mut four_bytes = Vec::new();
        two.encode(config, config.quality(), &rgb, &mut two_bytes)?;
        four.encode(config, config.quality(), &rgb, &mut four_bytes)?;
        let mut two_times = Vec::with_capacity(SAMPLES);
        let mut four_times = Vec::with_capacity(SAMPLES);
        for sample in 0..SAMPLES {
            // Alternate order to avoid giving one implementation all the
            // cold/warm scheduling and clock effects.
            if sample.is_multiple_of(2) {
                old_times.push(timed(reference, config, &rgb, &mut old)?);
                new_times.push(timed(current, config, &rgb, &mut new)?);
                two_times.push(timed(
                    |c, r, b| two.encode(c, c.quality(), r, b),
                    config,
                    &rgb,
                    &mut two_bytes,
                )?);
                four_times.push(timed(
                    |c, r, b| four.encode(c, c.quality(), r, b),
                    config,
                    &rgb,
                    &mut four_bytes,
                )?);
            } else {
                four_times.push(timed(
                    |c, r, b| four.encode(c, c.quality(), r, b),
                    config,
                    &rgb,
                    &mut four_bytes,
                )?);
                two_times.push(timed(
                    |c, r, b| two.encode(c, c.quality(), r, b),
                    config,
                    &rgb,
                    &mut two_bytes,
                )?);
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
        let mut expected = Vec::new();
        Decoder::default().decode_into(config, &new, &mut expected)?;
        for payload in [&two_bytes, &four_bytes] {
            let mut actual = Vec::new();
            Decoder::default().decode_into(config, payload, &mut actual)?;
            ensure!(
                actual == expected,
                "Parallel JPEG pixels differ from serial"
            );
        }
        metrics("image_reference", height, old.len(), old_times);
        metrics("bounded_rust_jpeg", height, new.len(), new_times);
        metrics("parallel_2", height, two_bytes.len(), two_times);
        metrics("parallel_4", height, four_bytes.len(), four_times);
        profile_stages(config, frame)?;
    }
    println!("encoder_check=passed; local generated pixels only, no capture or network.");
    Ok(())
}

fn profile_stages(config: VideoConfig, frame: image::RgbaImage) -> Result<()> {
    let config = config.with_mode(CodecMode::Compact);
    let mut changed = frame.clone();
    // Alternate two dense, substantially different images. The conversion
    // and exact tile attempts use the production implementation.
    for pixel in changed.pixels_mut() {
        pixel.0[0] ^= 0x80;
        pixel.0[1] ^= 0x40;
    }
    let mut encoder = Encoder::new(config);
    let mut decoder = Decoder::default();
    let mut bytes = Vec::new();
    let mut pixels = Vec::new();
    let mut resize = Vec::with_capacity(SAMPLES);
    let mut tiles = Vec::with_capacity(SAMPLES);
    let mut jpeg = Vec::with_capacity(SAMPLES);
    let mut total = Vec::with_capacity(SAMPLES);
    let mut jpeg_records = 0;
    for i in 0..=SAMPLES {
        let input = if i.is_multiple_of(2) {
            &frame
        } else {
            &changed
        };
        let started = Instant::now();
        let (record, stages) = encoder.encode_profiled(input, bytes)?;
        let elapsed = started.elapsed();
        let Capture::Frame(data) = record else {
            anyhow::bail!("Expected changed diagnostic frame");
        };
        decoder.decode_into(config, &data, &mut pixels)?;
        ensure!(
            pixels.len() == (config.width() * config.height()) as usize,
            "Decoded geometry changed"
        );
        if i > 0 {
            jpeg_records += usize::from(data[0] == JPEG_TAG);
            resize.push(stages.resize);
            tiles.push(stages.tiles);
            jpeg.push(stages.jpeg);
            total.push(elapsed);
        }
        bytes = data;
    }
    println!(
        "profile_height={}, jpeg_workers={}, jpeg_records={jpeg_records}, samples={SAMPLES}",
        config.height(),
        encoder.jpeg_workers()
    );
    for (stage, mut times) in [
        ("resize_rgb", resize),
        ("tiles_compare", tiles),
        ("jpeg", jpeg),
        ("total_encode", total),
    ] {
        times.sort_unstable();
        println!(
            "stage={stage}, height={}, p50_ms={:.3}, worst_ms={:.3}",
            config.height(),
            times[SAMPLES / 2 - 1].as_secs_f64() * 1000.0,
            times[SAMPLES - 1].as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
