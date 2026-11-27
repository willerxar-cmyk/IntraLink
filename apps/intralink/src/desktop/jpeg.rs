//! Portable baseline JPEG writer with explicit 4:4:4 chroma and a hard
//! output-length bound. The optional dependency feature performs its own
//! AVX2 runtime dispatch; the application does not execute unsafe code.
use std::io::{self, Write};

use anyhow::{Context, Result, ensure};
use intralink_core::{
    desktop::{MAX_FRAME_BYTES, VideoConfig},
    media::JPEG_TAG,
};
use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

pub(super) struct FrameWriter<'a> {
    pub(super) bytes: &'a mut Vec<u8>,
    pub(super) limit: usize,
}

impl Write for FrameWriter<'_> {
    #[inline]
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.write_all(bytes)?;
        Ok(bytes.len())
    }

    #[inline]
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        let end = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|&end| end <= self.limit)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "JPEG exceeds frame limit")
            })?;
        if end > self.bytes.capacity() {
            // Reuse the returned transport buffer. Grow geometrically but
            // request no more than the permitted frame length.
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(4096)
                .max(end)
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(io::Error::other)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn encode_rgb(
    config: VideoConfig,
    quality: u8,
    rgb: &[u8],
    output: &mut Vec<u8>,
) -> Result<()> {
    ensure!(
        (35..=config.quality()).contains(&quality),
        "JPEG quality outside approved range"
    );
    let (width, height) = (
        u16::try_from(config.width())?,
        u16::try_from(config.height())?,
    );
    encode_dimensions(width, height, quality, rgb, output)
}

/// Used only for validated full frames or local MCU-aligned strips.
pub(super) fn encode_dimensions(
    width: u16,
    height: u16,
    quality: u8,
    rgb: &[u8],
    output: &mut Vec<u8>,
) -> Result<()> {
    ensure!(
        width != 0 && height != 0 && rgb.len() == usize::from(width) * usize::from(height) * 3,
        "Invalid JPEG canvas length"
    );
    ensure!((35..=90).contains(&quality), "Invalid JPEG quality");
    output.clear();
    output.push(JPEG_TAG);
    let result = {
        let mut encoder = Encoder::new(
            FrameWriter {
                bytes: output,
                limit: MAX_FRAME_BYTES,
            },
            quality,
        );
        // The old Image encoder used 4:4:4 at every quality. Do not silently
        // trade away chroma detail or change the chosen quantization quality.
        encoder.set_sampling_factor(SamplingFactor::F_1_1);
        encoder.encode(rgb, width, height, ColorType::Rgb)
    };
    if let Err(error) = result {
        output.clear();
        return Err(error).context("Bounded software JPEG encode failed");
    }
    ensure!(
        output.len() > 1 && output.len() <= MAX_FRAME_BYTES,
        "Invalid encoded JPEG length"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ExtendedColorType, ImageFormat, codecs::jpeg::JpegEncoder};

    fn pattern(width: u32, height: u32) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                let (tx, ty) = if y < height / 4 {
                    (x, y)
                } else {
                    (x / 8, y / 8)
                };
                let noise = ((tx * 17 + ty * 29) ^ (tx * ty)) as u8;
                pixels.extend_from_slice(&[
                    (x / 8) as u8 ^ noise,
                    (y / 4) as u8 ^ (noise / 2),
                    (x / 16 + y / 16) as u8,
                ]);
            }
        }
        pixels
    }

    fn decoded(bytes: &[u8], config: VideoConfig) -> Result<Vec<u8>> {
        let image =
            image::load_from_memory_with_format(&bytes[1..], ImageFormat::Jpeg)?.into_rgb8();
        assert_eq!(image.dimensions(), (config.width(), config.height()));
        Ok(image.into_raw())
    }

    fn mse_by_channel(source: &[u8], result: &[u8]) -> [f64; 3] {
        let mut squared = [0u64; 3];
        for (a, b) in source.chunks_exact(3).zip(result.chunks_exact(3)) {
            for i in 0..3 {
                squared[i] += u64::from(a[i].abs_diff(b[i])).pow(2);
            }
        }
        squared.map(|value| value as f64 / (source.len() / 3) as f64)
    }

    // The generated JPEG is trusted test output. Parse only pre-SOS segments
    // to independently verify baseline sampling and quantization tables.
    fn headers(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        ensure!(
            bytes.starts_with(&[JPEG_TAG, 0xff, 0xd8]),
            "Missing JPEG SOI"
        );
        let mut offset = 3;
        let mut sof = Vec::new();
        let mut dqt = Vec::new();
        while offset + 4 <= bytes.len() {
            ensure!(bytes[offset] == 0xff, "Invalid generated marker");
            let marker = bytes[offset + 1];
            if marker == 0xda {
                break;
            }
            let length = usize::from(u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]));
            ensure!(
                length >= 2 && offset + 2 + length <= bytes.len(),
                "Invalid generated segment"
            );
            let segment = &bytes[offset + 4..offset + 2 + length];
            match marker {
                0xc0 => sof.extend_from_slice(segment),
                0xdb => dqt.extend_from_slice(segment),
                _ => {}
            }
            offset += 2 + length;
        }
        ensure!(
            sof.len() == 15 && dqt.len() == 130,
            "Expected baseline three-component JPEG and two quantization tables"
        );
        Ok((sof, dqt))
    }

    #[test]
    fn preserves_baseline_sampling_quantization_and_image_quality() -> Result<()> {
        let config = VideoConfig::new(540, 60, 90, 30).unwrap();
        let rgb = pattern(config.width(), config.height());
        for quality in [35, 75, 90] {
            let mut original = vec![JPEG_TAG];
            JpegEncoder::new_with_quality(&mut original, quality).encode(
                &rgb,
                config.width(),
                config.height(),
                ExtendedColorType::Rgb8,
            )?;
            let mut candidate = Vec::new();
            encode_rgb(config, quality, &rgb, &mut candidate)?;
            let (old_sof, old_dqt) = headers(&original)?;
            let (new_sof, new_dqt) = headers(&candidate)?;
            // Component identifiers may differ; the dimensions, precision
            // and three 1x1 sampling factors must match.
            assert_eq!(&new_sof[..6], &old_sof[..6]);
            for index in [7, 10, 13] {
                assert_eq!(new_sof[index], 0x11);
                assert_eq!(new_sof[index], old_sof[index]);
            }
            assert_eq!(
                new_dqt, old_dqt,
                "Quantization changed at quality {quality}"
            );
            let old_mse = mse_by_channel(&rgb, &decoded(&original, config)?);
            let new_mse = mse_by_channel(&rgb, &decoded(&candidate, config)?);
            println!(
                "quality={quality}, old_bytes={}, new_bytes={}, old_mse={old_mse:?}, new_mse={new_mse:?}",
                original.len(),
                candidate.len()
            );
            for i in 0..3 {
                assert!(
                    new_mse[i] <= old_mse[i] * 1.08 + 0.5,
                    "Image error regressed at quality {quality}, channel {i}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn bounded_writer_rejects_before_mutation_and_reuses_capacity() -> Result<()> {
        let mut bytes = vec![1, 2, 3];
        let mut sink = FrameWriter {
            bytes: &mut bytes,
            limit: 8,
        };
        sink.write_all(&[4, 5, 6])?;
        assert!(sink.write_all(&[7, 8, 9]).is_err());
        assert_eq!(sink.bytes, &[1, 2, 3, 4, 5, 6]);
        sink.write_all(&[7, 8])?;
        assert_eq!(sink.bytes.len(), 8);
        assert!(sink.write_all(&[9]).is_err());
        let config = VideoConfig::new(540, 60, 75, 30).unwrap();
        let rgb = vec![50; config.width() as usize * config.height() as usize * 3];
        let mut output = Vec::new();
        encode_rgb(config, 75, &rgb, &mut output)?;
        let capacity = output.capacity();
        let pointer = output.as_ptr();
        encode_rgb(config, 75, &rgb, &mut output)?;
        assert_eq!(output.capacity(), capacity);
        assert_eq!(output.as_ptr(), pointer);
        assert!(encode_rgb(config, 76, &rgb, &mut output).is_err());
        assert!(encode_rgb(config, 75, &rgb[..rgb.len() - 1], &mut output).is_err());
        Ok(())
    }
}

#[cfg(test)]
#[test]
fn oversized_dense_jpeg_leaves_no_partial_record() -> Result<()> {
    let config = VideoConfig::new(1080, 60, 90, 30).unwrap();
    let mut rgb = vec![0; (config.width() * config.height() * 3) as usize];
    let mut seed = 0x1234_5678u32;
    for byte in &mut rgb {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *byte = (seed >> 24) as u8;
    }
    let mut output = Vec::new();
    assert!(encode_rgb(config, 90, &rgb, &mut output).is_err());
    assert!(output.is_empty());
    assert!(output.capacity() <= MAX_FRAME_BYTES);
    Ok(())
}
