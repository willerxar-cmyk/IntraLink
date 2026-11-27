//! Shared bounded software decoder for desktop viewers and acceptance tests.
use anyhow::{Result, ensure};
use image::{ImageFormat, ImageReader, Limits};
use intralink_core::{
    adaptive::{ViewProtocol, config_at_height, is_keyframe},
    desktop::{MAX_FRAME_BYTES, VideoConfig},
    media::{self, JPEG_TAG, LOSSLESS_KEYFRAME_TAG, TILE_TAG},
};
use std::io::Cursor;

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
