//! Shared bounded software decoder for desktop viewers and acceptance tests.
use anyhow::{Result, ensure};
use image::{ColorType, ImageDecoder, Limits, codecs::jpeg::JpegDecoder};
use intralink_core::{
    adaptive::{ViewProtocol, config_at_height, is_keyframe},
    desktop::{MAX_FRAME_BYTES, VideoConfig},
    media::{self, JPEG_TAG, LOSSLESS_KEYFRAME_TAG, TILE_TAG},
};
use std::io::Cursor;

/// Reuse the presentation allocation across frames; never decode repeat records.
fn decode_jpeg(
    config: VideoConfig,
    data: &[u8],
    pixels: &mut Vec<u32>,
    scratch: &mut Vec<u8>,
) -> Result<()> {
    ensure!(
        !data.is_empty() && data.len() <= MAX_FRAME_BYTES,
        "Invalid JPEG frame size"
    );
    let mut decoder = JpegDecoder::new(Cursor::new(data))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(config.width());
    limits.max_image_height = Some(config.height());
    limits.max_alloc = Some(32 * 1024 * 1024);
    decoder.set_limits(limits)?;
    ensure!(
        decoder.dimensions() == (config.width(), config.height()),
        "Unexpected frame dimensions"
    );
    let color = decoder.color_type();
    let channels = match color {
        ColorType::L8 => 1,
        ColorType::La8 => 2,
        ColorType::Rgb8 => 3,
        ColorType::Rgba8 => 4,
        _ => anyhow::bail!("Unsupported JPEG output format"),
    };
    let count = (config.width() * config.height()) as usize;
    let length = count * channels;
    ensure!(
        decoder.total_bytes() == length as u64 && length <= 32 * 1024 * 1024,
        "Invalid decoded JPEG allocation"
    );
    scratch.resize(length, 0);
    // Never mutate the presentation canvas until the full bounded decode
    // succeeds. Reuse the same allocation on subsequent JPEG frames.
    decoder.read_image(scratch)?;
    pixels.resize(count, 0);
    if channels <= 2 {
        for (source, target) in scratch.chunks_exact(channels).zip(pixels.iter_mut()) {
            *target = u32::from(source[0]) * 0x010101;
        }
    } else {
        for (source, target) in scratch.chunks_exact(channels).zip(pixels.iter_mut()) {
            *target =
                (u32::from(source[0]) << 16) | (u32::from(source[1]) << 8) | u32::from(source[2]);
        }
    }
    Ok(())
}

/// Stateful decode: no encoded update may be skipped. A malformed update or
/// a delta before the first keyframe terminates the session.
#[derive(Default)]
pub struct Decoder {
    config: Option<VideoConfig>,
    jpeg_scratch: Vec<u8>,
}

impl Decoder {
    pub fn decode_into(
        &mut self,
        config: VideoConfig,
        data: &[u8],
        pixels: &mut Vec<u32>,
    ) -> Result<()> {
        self.decode_into_protocol(config, data, pixels, ViewProtocol::Fixed)
    }

    pub fn decode_into_protocol(
        &mut self,
        config: VideoConfig,
        data: &[u8],
        pixels: &mut Vec<u32>,
        protocol: ViewProtocol,
    ) -> Result<()> {
        ensure!(
            !data.is_empty() && data.len() <= MAX_FRAME_BYTES,
            "Invalid media record size"
        );
        if let Some(previous) = self.config
            && previous != config
        {
            let ceiling = if config.height() >= previous.height() {
                config
            } else {
                previous
            };
            ensure!(
                protocol.is_adaptive()
                    && config_at_height(ceiling, previous.height() as u16) == Some(previous)
                    && config_at_height(ceiling, config.height() as u16) == Some(config)
                    && is_keyframe(config.mode(), data),
                "Media configuration changed without an adaptive keyframe"
            );
        }
        match data[0] {
            JPEG_TAG => decode_jpeg(config, &data[1..], pixels, &mut self.jpeg_scratch)?,
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
    use image::{ImageFormat, ImageReader};

    #[test]
    fn reused_jpeg_canvas_matches_reference_and_failure_is_atomic() -> Result<()> {
        let config = VideoConfig::new(540, 60, 75, 15).unwrap();
        let (w, h) = (config.width() as u16, config.height() as u16);
        let mut decoder = Decoder::default();
        let mut pixels = Vec::new();
        for color in [jpeg_encoder::ColorType::Rgb, jpeg_encoder::ColorType::Luma] {
            let channels = if color == jpeg_encoder::ColorType::Rgb {
                3
            } else {
                1
            };
            let source: Vec<u8> = (0..usize::from(w) * usize::from(h) * channels)
                .map(|i| (i * 7 + i / 97) as u8)
                .collect();
            let mut payload = vec![JPEG_TAG];
            jpeg_encoder::Encoder::new(&mut payload, 75).encode(&source, w, h, color)?;
            let reference = ImageReader::with_format(Cursor::new(&payload[1..]), ImageFormat::Jpeg)
                .decode()?
                .into_rgb8();
            decoder.decode_into(config, &payload, &mut pixels)?;
            for (rgb, &pixel) in reference.as_raw().chunks_exact(3).zip(&pixels) {
                assert_eq!(
                    pixel,
                    (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2])
                );
            }
            let address = decoder.jpeg_scratch.as_ptr();
            decoder.decode_into(config, &payload, &mut pixels)?;
            assert_eq!(decoder.jpeg_scratch.as_ptr(), address);
            let before = pixels.clone();
            assert!(
                decoder
                    .decode_into(config, &[JPEG_TAG, 0xff, 0xd8], &mut pixels)
                    .is_err()
            );
            assert_eq!(pixels, before);
        }
        Ok(())
    }
}
