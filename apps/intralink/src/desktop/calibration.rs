//! Local software decoder calibration. Only generated pixels and the real
//! production decoder are used; no screen capture, network or vault access.
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use image::{ExtendedColorType, codecs::jpeg::JpegEncoder};
use intralink_core::{
    calibration::{DecodeTier, recommend},
    desktop::{CodecMode, MAX_FRAME_BYTES, VideoConfig},
    media::{self, Delta, JPEG_TAG, MAX_DELTA_BYTES},
    profile_limits::ReceiverLimits,
};

use super::codec::Decoder;

const SAMPLES: usize = 6;
const LOSSLESS_BUDGET: usize = 128 * 1024;

#[derive(clap::Args)]
pub struct ProfileCheckArgs {
    /// Number of simultaneously selected workspace slots.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=8))]
    slots: u8,
    /// Probe optional native Windows adapters separately from software decode.
    #[arg(long)]
    hardware: bool,
}

impl ProfileCheckArgs {
    pub fn run(self) -> Result<()> {
        if self.hardware {
            #[cfg(not(all(target_os = "windows", feature = "wgc-preview")))]
            anyhow::bail!("Native adapter probe requires Windows wgc-preview");
        }
        let measured = measure(CodecMode::Compact, ReceiverLimits::MAX)?;
        let limits = measured.recommend(ReceiverLimits::MAX, usize::from(self.slots))?;
        println!(
            "recommended_height={}, recommended_fps={}, slots={}, software_only=true",
            limits.height(),
            limits.fps(),
            self.slots
        );
        #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
        if self.hardware {
            intralink_d3d11::probe_video_capabilities()?;
        }
        println!(
            "Generated local fixtures only; no remote performance or hardware codec qualification."
        );
        Ok(())
    }
}

pub(super) struct Calibration(Vec<DecodeTier>);

impl Calibration {
    pub(super) fn recommend(
        &self,
        ceiling: ReceiverLimits,
        slots: usize,
    ) -> Result<ReceiverLimits> {
        recommend(&self.0, ceiling, slots)
            .context("Measured decode budget cannot support this receiver profile")
    }
}

fn generated_jpeg(config: VideoConfig) -> Result<Vec<u8>> {
    let (width, height) = (config.width(), config.height());
    let mut rgb = vec![0; (width * height * 3) as usize];
    for (i, pixel) in rgb.chunks_exact_mut(3).enumerate() {
        let x = i as u32 % width;
        let y = i as u32 / width;
        // Keep a region of high-frequency detail while the rest resembles
        // block/gradient desktop content. At quality 90 the fixture itself
        // must fit the existing 2 MiB protocol ceiling at every tier.
        let (tx, ty) = if y < height / 4 {
            (x, y)
        } else {
            (x / 8, y / 8)
        };
        let texture = ((tx * 17 + ty * 29) ^ (tx * ty)) as u8;
        pixel.copy_from_slice(&[
            (x / 8) as u8 ^ texture,
            (y / 4) as u8 ^ (texture / 2),
            (x / 16 + y / 16) as u8,
        ]);
        if x < 64 && y < 64 {
            pixel.copy_from_slice(&[70, 120, 180]);
        }
    }
    let mut bytes = vec![JPEG_TAG];
    // Encode at the maximum allowed JPEG quality. Fixture construction is
    // excluded from decode timing; no claim of a worst-case hostile bitstream.
    JpegEncoder::new_with_quality(&mut bytes, 90).encode(
        &rgb,
        width,
        height,
        ExtendedColorType::Rgb8,
    )?;
    ensure!(
        (2..=MAX_FRAME_BYTES).contains(&bytes.len()),
        "Calibration JPEG exceeds protocol bound"
    );
    Ok(bytes)
}

fn decode_samples(
    config: VideoConfig,
    bytes: &[u8],
    base: Option<&[u8]>,
) -> Result<(Duration, Vec<u32>)> {
    let mut decoder = Decoder::default();
    let mut pixels = Vec::new();
    if let Some(base) = base {
        decoder.decode_into(config, base, &mut pixels)?;
    }
    // Warm-up once to establish allocations. Absolute tiles may be applied
    // repeatedly; a fresh keyframe and the production validation run each time.
    decoder.decode_into(config, bytes, &mut pixels)?;
    let expected = (config.width() * config.height()) as usize;
    ensure!(
        pixels.len() == expected,
        "Calibration decode geometry mismatch"
    );
    let mut worst = Duration::ZERO;
    for _ in 0..SAMPLES {
        let start = Instant::now();
        decoder.decode_into(config, bytes, &mut pixels)?;
        let elapsed = start.elapsed();
        ensure!(
            pixels.len() == expected,
            "Calibration decode geometry changed"
        );
        worst = worst.max(elapsed);
    }
    ensure!(!worst.is_zero(), "Local clock cannot measure decoder work");
    Ok((worst, pixels))
}

fn tier(mode: CodecMode, height: u16) -> Result<DecodeTier> {
    let config = VideoConfig::new(height, 60, 90, 30)
        .context("Invalid calibration profile")?
        .with_mode(mode);
    let jpeg = generated_jpeg(config)?;
    let (jpeg_time, pixels) = decode_samples(config, &jpeg, None)?;
    let marker = pixels[(config.width() * 24 + 24) as usize];
    for (shift, expected) in [(16, 70u8), (8, 120), (0, 180)] {
        ensure!(
            ((marker >> shift) as u8).abs_diff(expected) <= 5,
            "Calibration JPEG marker mismatch"
        );
    }
    let mut worst = jpeg_time;
    let mut compact_time = None;
    let mut tile_time = None;
    if mode.supports_tiles() {
        let (w, h) = (config.width() as usize, config.height() as usize);
        let mut rgb = vec![0; w * h * 3];
        if mode.supports_compact() {
            for (i, pixel) in rgb.chunks_exact_mut(3).enumerate() {
                let tile = (i / w / 32 + i % w / 32) % 2;
                pixel.copy_from_slice(if tile == 0 {
                    &[41, 83, 129]
                } else {
                    &[89, 137, 173]
                });
            }
            let mut key = Vec::new();
            ensure!(
                media::encode_keyframe(config, &rgb, &mut key, LOSSLESS_BUDGET)
                    .map_err(|e| anyhow::anyhow!("Calibration keyframe: {e:?}"))?,
                "Calibration keyframe did not fit"
            );
            let (time, canvas) = decode_samples(config, &key, None)?;
            ensure!(
                canvas
                    .iter()
                    .zip(rgb.chunks_exact(3))
                    .all(|(&pixel, src)| pixel
                        == (u32::from(src[0]) << 16 | u32::from(src[1]) << 8 | u32::from(src[2]))),
                "Calibration keyframe pixels mismatch"
            );
            worst = worst.max(time);
            compact_time = Some(time);
        }
        // Exercise dense absolute tile updates under the actual wire cap.
        let previous = rgb.clone();
        for y in 0..32.min(h) {
            for x in 0..w.min(1024) {
                let start = (y * w + x) * 3;
                rgb[start..start + 3].copy_from_slice(&[
                    (x * 13 + y * 17) as u8,
                    (x * y) as u8,
                    211,
                ]);
            }
        }
        // The peer may select Tiles instead of Compact, so measure both
        // allowed tile representations rather than infer equivalent cost.
        for tile_mode in [CodecMode::Tiles, CodecMode::Compact] {
            if tile_mode == CodecMode::Compact && !mode.supports_compact() {
                continue;
            }
            let config = config.with_mode(tile_mode);
            let mut delta = Vec::new();
            ensure!(
                media::encode_delta(config, &previous, &rgb, &mut delta, MAX_DELTA_BYTES)
                    .map_err(|e| anyhow::anyhow!("Calibration delta: {e:?}"))?
                    == Delta::Encoded,
                "Calibration tile budget exhausted"
            );
            let (time, canvas) = decode_samples(config, &delta, Some(&jpeg))?;
            ensure!(
                canvas[0] == 211 && canvas[w + 1] == 0x1e01d3,
                "Calibration tile marker mismatch"
            );
            tile_time = Some(tile_time.map_or(time, |prior: Duration| prior.max(time)));
            worst = worst.max(time);
        }
    }
    let ms = |time: Duration| time.as_secs_f64() * 1000.0;
    println!(
        "decode_tier={height}p, samples_per_fixture={SAMPLES}, jpeg_worst_ms={:.3}, compact_worst_ms={:?}, tiles_worst_ms={:?}, worst_ms={:.3}",
        ms(jpeg_time),
        compact_time.map(ms),
        tile_time.map(ms),
        ms(worst)
    );
    Ok(DecodeTier { height, worst })
}

pub(super) fn measure(mode: CodecMode, ceiling: ReceiverLimits) -> Result<Calibration> {
    println!(
        "Local software decoder calibration: mode={mode:?}, logical_cpus={:?}",
        std::thread::available_parallelism().ok().map(|n| n.get())
    );
    let mut tiers = Vec::with_capacity(3);
    for height in [1080, 720, 540] {
        if height <= ceiling.height() {
            tiers.push(tier(mode, height)?);
        }
    }
    Ok(Calibration(tiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_jpeg_fits_wire_limit_at_every_resolution() -> Result<()> {
        for height in [540, 720, 1080] {
            let config = VideoConfig::new(height, 60, 90, 30).unwrap();
            let bytes = generated_jpeg(config)?;
            assert!((2..=MAX_FRAME_BYTES).contains(&bytes.len()));
        }
        Ok(())
    }

    #[test]
    fn real_software_fixtures_round_trip_without_timing_assumptions() -> Result<()> {
        for mode in [CodecMode::Jpeg, CodecMode::Tiles, CodecMode::Compact] {
            let sample = tier(mode, 540)?;
            assert_eq!(sample.height, 540);
            assert!(!sample.worst.is_zero());
        }
        Ok(())
    }
}
