//! Reusable RGB canvases, JPEG or lossless keyframes and ordered Rust tile updates.
use std::io::Cursor;

use anyhow::{Result, ensure};
use image::{
    ExtendedColorType, ImageFormat, ImageReader, Limits, RgbaImage, codecs::jpeg::JpegEncoder,
};
use intralink_core::{
    desktop::{MAX_FRAME_BYTES, VideoConfig},
    media::{self, Delta, JPEG_TAG, LOSSLESS_KEYFRAME_TAG, TILE_TAG},
};
use intralink_transport::desktop::Capture;

pub const MAX_SOURCE_PIXELS: u64 = 33_177_600;
// Bound the extra work and wire size of a lossless keyframe attempt. This
// policy favors exact desktop UI content, not necessarily the smallest JPEG.
const LOSSLESS_ATTEMPT_BUDGET: usize = 128 * 1024;
const MIN_DELTA_BUDGET: usize = 32 * 1024;

pub fn validate_source_size(width: u32, height: u32) -> Result<()> {
    ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_SOURCE_PIXELS,
        "Unsupported source dimensions"
    );
    Ok(())
}

/// Keep two bounded output-size canvases rather than a second native screenshot.
pub struct Encoder {
    config: VideoConfig,
    resizer: super::resize::CanvasResizer,
    scratch: Vec<u8>,
    previous: Vec<u8>,
    has_previous: bool,
    last_keyframe_size: usize,
    deltas_since_keyframe: u16,
}

impl Encoder {
    pub fn new(config: VideoConfig) -> Self {
        let size = (config.width() * config.height() * 3) as usize;
        Self {
            config,
            resizer: super::resize::CanvasResizer::default(),
            scratch: vec![0; size],
            previous: vec![0; size],
            has_previous: false,
            last_keyframe_size: 0,
            deltas_since_keyframe: 0,
        }
    }

    pub fn encode(&mut self, frame: &RgbaImage, mut jpeg: Vec<u8>) -> Result<Capture> {
        validate_source_size(frame.width(), frame.height())?;
        let (width, height) = (self.config.width(), self.config.height());
        self.resizer
            .fill_rgb(frame, &mut self.scratch, (width, height))?;
        jpeg.clear();
        // Hybrid is explicitly selected by both peers. Refresh after a bounded
        // number of changed records; static screens still use repeat records.
        let delta = if self.config.mode().supports_tiles()
            && self.has_previous
            && self.deltas_since_keyframe < 120
        {
            let budget = self
                .last_keyframe_size
                .clamp(MIN_DELTA_BUDGET, media::MAX_DELTA_BYTES);
            media::encode_delta(
                self.config,
                &self.previous,
                &self.scratch,
                &mut jpeg,
                budget,
            )
            .map_err(|err| anyhow::anyhow!("Tile encoder failed: {err:?}"))?
        } else if self.has_previous && self.scratch == self.previous {
            Delta::Unchanged
        } else {
            Delta::KeyframeRequired
        };
        match delta {
            Delta::Unchanged => return Ok(Capture::Repeat(jpeg)),
            Delta::Encoded => self.deltas_since_keyframe += 1,
            Delta::KeyframeRequired => {
                let exact = self.config.mode().supports_compact()
                    && media::encode_keyframe(
                        self.config,
                        &self.scratch,
                        &mut jpeg,
                        LOSSLESS_ATTEMPT_BUDGET,
                    )
                    .map_err(|err| anyhow::anyhow!("Lossless keyframe encoder failed: {err:?}"))?;
                if !exact {
                    jpeg.clear();
                    jpeg.push(JPEG_TAG);
                    JpegEncoder::new_with_quality(&mut jpeg, self.config.quality()).encode(
                        &self.scratch,
                        width,
                        height,
                        ExtendedColorType::Rgb8,
                    )?;
                }
                ensure!(
                    jpeg.len() > 1 && jpeg.len() <= MAX_FRAME_BYTES,
                    "Encoded frame exceeds wire limit; select lower quality or resolution"
                );
                self.last_keyframe_size = jpeg.len();
                self.deltas_since_keyframe = 0;
            }
        }
        std::mem::swap(&mut self.scratch, &mut self.previous);
        self.has_previous = true;
        Ok(Capture::Frame(jpeg))
    }
}

/// Reuse the presentation allocation across frames; never decode repeat records.
fn decode_jpeg(config: VideoConfig, data: &[u8], pixels: &mut Vec<u32>) -> Result<()> {
    ensure!(
        !data.is_empty() && data.len() <= MAX_FRAME_BYTES,
        "Invalid JPEG frame size"
    );
    let mut reader = ImageReader::with_format(Cursor::new(data), ImageFormat::Jpeg);
    let mut limits = Limits::default();
    limits.max_image_width = Some(config.width());
    limits.max_image_height = Some(config.height());
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let rgb = reader.decode()?.into_rgb8();
    ensure!(
        rgb.dimensions() == (config.width(), config.height()),
        "Unexpected frame dimensions"
    );
    pixels.resize((config.width() * config.height()) as usize, 0);
    for (source, target) in rgb.as_raw().chunks_exact(3).zip(pixels.iter_mut()) {
        *target = (u32::from(source[0]) << 16) | (u32::from(source[1]) << 8) | u32::from(source[2]);
    }
    Ok(())
}

/// Stateful decode: no encoded update may be skipped. A malformed update or
/// a delta before the first keyframe terminates the session.
#[derive(Default)]
pub struct Decoder {
    config: Option<VideoConfig>,
}

impl Decoder {
    pub fn decode_into(
        &mut self,
        config: VideoConfig,
        data: &[u8],
        pixels: &mut Vec<u32>,
    ) -> Result<()> {
        ensure!(
            !data.is_empty() && data.len() <= MAX_FRAME_BYTES,
            "Invalid media record size"
        );
        ensure!(
            self.config.is_none_or(|previous| previous == config),
            "Media configuration changed"
        );
        match data[0] {
            JPEG_TAG => decode_jpeg(config, &data[1..], pixels)?,
            LOSSLESS_KEYFRAME_TAG => media::apply_keyframe(config, data, pixels)
                .map_err(|err| anyhow::anyhow!("Invalid lossless keyframe: {err:?}"))?,
            TILE_TAG => {
                ensure!(
                    config.mode().supports_tiles() && self.config == Some(config),
                    "Unnegotiated tile update or missing keyframe"
                );
                media::apply_delta(config, data, pixels)
                    .map_err(|err| anyhow::anyhow!("Invalid tile update: {err:?}"))?;
            }
            _ => anyhow::bail!("Unknown media record type"),
        }
        self.config = Some(config);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{RgbImage, Rgba};
    use intralink_core::desktop::CodecMode;

    fn config() -> VideoConfig {
        VideoConfig::new(540, 15, 75, 30).unwrap()
    }

    fn changed(capture: Capture) -> Vec<u8> {
        match capture {
            Capture::Frame(data) => data,
            _ => panic!("Expected a changed frame"),
        }
    }

    #[test]
    fn codec_round_trip_and_letterbox() -> Result<()> {
        let config = config();
        let mut encoder = Encoder::new(config);
        let fixture = RgbaImage::from_pixel(120, 120, Rgba([230, 20, 30, 255]));
        let jpeg = changed(encoder.encode(&fixture, Vec::new())?);
        assert_eq!(&jpeg[..3], &[JPEG_TAG, 0xff, 0xd8]);
        let mut pixels = Vec::new();
        Decoder::default().decode_into(config, &jpeg, &mut pixels)?;
        assert_eq!(pixels.len(), (config.width() * config.height()) as usize);
        let center = pixels[(config.width() * (config.height() / 2) + config.width() / 2) as usize];
        assert!((center >> 16) & 255 > 200);
        assert!(pixels[0] < 0x101010);
        Ok(())
    }

    #[test]
    fn static_pixels_skip_encoding_and_change_triggers_new_jpeg() -> Result<()> {
        let config = config();
        let mut encoder = Encoder::new(config);
        let mut frame =
            RgbaImage::from_pixel(config.width(), config.height(), Rgba([0, 0, 0, 255]));
        let jpeg = changed(encoder.encode(&frame, Vec::new())?);
        let capacity = jpeg.capacity();
        let mut recycled = match encoder.encode(&frame, jpeg)? {
            Capture::Repeat(data) => data,
            _ => panic!("Unchanged screen must not encode"),
        };
        assert!(recycled.is_empty());
        assert_eq!(recycled.capacity(), capacity);
        frame.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        recycled = changed(encoder.encode(&frame, recycled)?);
        let mut pixels = Vec::new();
        Decoder::default().decode_into(config, &recycled, &mut pixels)?;
        let ptr = pixels.as_ptr();
        Decoder::default().decode_into(config, &recycled, &mut pixels)?;
        assert_eq!(pixels.as_ptr(), ptr);
        Ok(())
    }

    #[test]
    fn hybrid_uses_ordered_tiles_after_keyframe() -> Result<()> {
        let config = config().with_mode(CodecMode::Tiles);
        let mut encoder = Encoder::new(config);
        let mut decoder = Decoder::default();
        let mut frame =
            RgbaImage::from_pixel(config.width(), config.height(), Rgba([0, 0, 0, 255]));
        let key = changed(encoder.encode(&frame, Vec::new())?);
        assert_eq!(key[0], JPEG_TAG);
        let mut pixels = Vec::new();
        decoder.decode_into(config, &key, &mut pixels)?;
        for value in [120, 240, 0] {
            for y in 0..32 {
                for x in 0..32 {
                    frame.put_pixel(x, y, Rgba([value, 0, 0, 255]));
                }
            }
            let patch = changed(encoder.encode(&frame, Vec::new())?);
            assert_eq!(patch.len(), 9);
            assert_eq!(patch[0], TILE_TAG);
            assert!(
                Decoder::default()
                    .decode_into(config, &patch, &mut Vec::new())
                    .is_err()
            );
            decoder.decode_into(config, &patch, &mut pixels)?;
            assert_eq!(pixels[0], u32::from(value) << 16);
            assert_eq!(pixels[32], 0);
        }
        Ok(())
    }

    #[test]
    fn compact_hybrid_decodes_palette_after_lossless_keyframe_and_rejects_mode_switch() -> Result<()>
    {
        let config = config().with_mode(CodecMode::Compact);
        let mut encoder = Encoder::new(config);
        let mut decoder = Decoder::default();
        let mut frame =
            RgbaImage::from_pixel(config.width(), config.height(), Rgba([0, 0, 0, 255]));
        let key = changed(encoder.encode(&frame, Vec::new())?);
        assert_eq!(key[0], LOSSLESS_KEYFRAME_TAG);
        let mut pixels = Vec::new();
        decoder.decode_into(config, &key, &mut pixels)?;
        for y in 0..32 {
            for x in 0..32 {
                if (x + y) % 2 == 0 {
                    frame.put_pixel(x, y, Rgba([20, 40, 60, 255]));
                }
            }
        }
        let patch = changed(encoder.encode(&frame, Vec::new())?);
        assert_eq!(patch.len(), 141);
        assert_eq!(patch[0], TILE_TAG);
        assert!(
            decoder
                .decode_into(config.with_mode(CodecMode::Tiles), &patch, &mut pixels)
                .is_err()
        );
        decoder.decode_into(config, &patch, &mut pixels)?;
        assert_eq!(pixels[0], 0x14283c);
        assert_eq!(pixels[1], 0);
        Ok(())
    }

    #[test]
    fn compact_exact_then_jpeg_then_exact_refresh() -> Result<()> {
        let config = config().with_mode(CodecMode::Compact);
        let mut encoder = Encoder::new(config);
        let mut decoder = Decoder::default();
        let mut pixels = Vec::new();
        let mut frame =
            RgbaImage::from_pixel(config.width(), config.height(), Rgba([17, 43, 199, 255]));
        let exact = changed(encoder.encode(&frame, Vec::new())?);
        assert_eq!(exact[0], LOSSLESS_KEYFRAME_TAG);
        decoder.decode_into(config, &exact, &mut pixels)?;
        assert!(pixels.iter().all(|pixel| *pixel == 0x112bc7));
        // Deterministically force a high-entropy source beyond the tile budget.
        let mut state = 0x1234_5678u32;
        for pixel in frame.pixels_mut() {
            for channel in &mut pixel.0[..3] {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                *channel = (state >> 24) as u8;
            }
        }
        let jpeg = changed(encoder.encode(&frame, exact)?);
        assert_eq!(jpeg[0], JPEG_TAG);
        decoder.decode_into(config, &jpeg, &mut pixels)?;
        let frame = RgbaImage::from_pixel(config.width(), config.height(), Rgba([29, 31, 37, 255]));
        // Force a periodic refresh to check the exact base after a lossy frame.
        encoder.deltas_since_keyframe = 120;
        let exact = changed(encoder.encode(&frame, jpeg)?);
        assert_eq!(exact[0], LOSSLESS_KEYFRAME_TAG);
        decoder.decode_into(config, &exact, &mut pixels)?;
        assert!(pixels.iter().all(|pixel| *pixel == 0x1d1f25));
        Ok(())
    }

    #[test]
    fn malformed_oversized_and_wrong_dimensions_are_rejected() -> Result<()> {
        let config = config();
        let mut pixels = Vec::new();
        assert!(
            Decoder::default()
                .decode_into(config, b"not jpeg", &mut pixels)
                .is_err()
        );
        assert!(
            Decoder::default()
                .decode_into(config, &[JPEG_TAG, 1, 2, 3], &mut pixels)
                .is_err()
        );
        assert!(
            Decoder::default()
                .decode_into(config, &vec![0; MAX_FRAME_BYTES + 1], &mut pixels)
                .is_err()
        );
        assert!(validate_source_size(0, 10).is_err());
        assert!(validate_source_size(u32::MAX, u32::MAX).is_err());
        let mut wrong = vec![JPEG_TAG];
        JpegEncoder::new_with_quality(&mut wrong, 75)
            .encode_image(&RgbImage::new(config.width() + 1, config.height()))?;
        assert!(
            Decoder::default()
                .decode_into(config, &wrong, &mut pixels)
                .is_err()
        );
        Ok(())
    }
}
