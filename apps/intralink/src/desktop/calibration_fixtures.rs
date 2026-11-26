//! Bounded generated fixtures shared read-only by calibration workers.
use anyhow::{Context, Result, ensure};
use image::{ExtendedColorType, codecs::jpeg::JpegEncoder};
use intralink_core::{
    desktop::{CodecMode, MAX_FRAME_BYTES, VideoConfig},
    media::{self, Delta, JPEG_TAG, MAX_DELTA_BYTES},
};

const LOSSLESS_BUDGET: usize = 128 * 1024;
const TILE_ROWS: usize = 32;
const TILE_WIDTH: usize = 1024;

#[derive(Clone, Copy)]
enum Kind {
    Jpeg,
    Keyframe,
    Delta,
}

pub(super) struct Fixture {
    pub(super) config: VideoConfig,
    pub(super) bytes: Vec<u8>,
    kind: Kind,
}

impl Fixture {
    pub(super) fn name(&self) -> &'static str {
        match self.kind {
            Kind::Jpeg => "jpeg",
            Kind::Keyframe => "compact_keyframe",
            Kind::Delta if self.config.mode() == CodecMode::Compact => "compact_delta",
            Kind::Delta => "tiles_delta",
        }
    }

    pub(super) fn verify(&self, pixels: &[u32], retained_pixel: Option<u32>) -> Result<()> {
        let (w, h) = (self.config.width() as usize, self.config.height() as usize);
        ensure!(pixels.len() == w * h, "Calibration geometry mismatch");
        match self.kind {
            Kind::Jpeg => {
                let marker = pixels[w * 24 + 24];
                for (shift, expected) in [(16, 70u8), (8, 120), (0, 180)] {
                    ensure!(
                        ((marker >> shift) as u8).abs_diff(expected) <= 5,
                        "Calibration JPEG marker mismatch"
                    );
                }
            }
            Kind::Keyframe => {
                ensure!(
                    pixels
                        .iter()
                        .enumerate()
                        .all(|(i, &pixel)| pixel == rgb32(key_color(w, i))),
                    "Calibration keyframe pixels mismatch"
                );
            }
            Kind::Delta => {
                for y in 0..TILE_ROWS.min(h) {
                    for x in 0..TILE_WIDTH.min(w) {
                        ensure!(
                            pixels[y * w + x] == rgb32(delta_color(x, y)),
                            "Calibration delta pixels mismatch"
                        );
                    }
                }
                ensure!(
                    retained_pixel == Some(pixels[TILE_ROWS * w]),
                    "Calibration delta changed untouched pixel"
                );
            }
        }
        Ok(())
    }

    pub(super) fn retained_pixel(&self, pixels: &[u32]) -> Option<u32> {
        matches!(self.kind, Kind::Delta).then(|| pixels[TILE_ROWS * self.config.width() as usize])
    }
}

pub(super) struct FixtureSet {
    pub(super) height: u16,
    pub(super) items: Vec<Fixture>,
}

impl FixtureSet {
    pub(super) fn new(mode: CodecMode, height: u16) -> Result<Self> {
        let config = VideoConfig::new(height, 60, 90, 30)
            .context("Invalid calibration profile")?
            .with_mode(mode);
        let mut items = vec![Fixture {
            config,
            bytes: generated_jpeg(config)?,
            kind: Kind::Jpeg,
        }];
        if mode.supports_tiles() {
            let (w, h) = (config.width() as usize, config.height() as usize);
            let mut rgb = vec![0; w * h * 3];
            if mode.supports_compact() {
                for (i, pixel) in rgb.chunks_exact_mut(3).enumerate() {
                    pixel.copy_from_slice(&key_color(w, i));
                }
                let mut bytes = Vec::new();
                ensure!(
                    media::encode_keyframe(config, &rgb, &mut bytes, LOSSLESS_BUDGET)
                        .map_err(|e| anyhow::anyhow!("Calibration keyframe: {e:?}"))?,
                    "Calibration keyframe did not fit"
                );
                items.push(Fixture {
                    config,
                    bytes,
                    kind: Kind::Keyframe,
                });
            }
            let previous = rgb.clone();
            for y in 0..TILE_ROWS.min(h) {
                for x in 0..TILE_WIDTH.min(w) {
                    let start = (y * w + x) * 3;
                    rgb[start..start + 3].copy_from_slice(&delta_color(x, y));
                }
            }
            for tile_mode in [CodecMode::Tiles, CodecMode::Compact] {
                if tile_mode == CodecMode::Compact && !mode.supports_compact() {
                    continue;
                }
                let config = config.with_mode(tile_mode);
                let mut bytes = Vec::new();
                ensure!(
                    media::encode_delta(config, &previous, &rgb, &mut bytes, MAX_DELTA_BYTES)
                        .map_err(|e| anyhow::anyhow!("Calibration delta: {e:?}"))?
                        == Delta::Encoded,
                    "Calibration delta budget exhausted"
                );
                items.push(Fixture {
                    config,
                    bytes,
                    kind: Kind::Delta,
                });
            }
        }
        Ok(Self { height, items })
    }

    pub(super) fn base(&self, index: usize) -> Option<&[u8]> {
        matches!(self.items[index].kind, Kind::Delta).then_some(self.items[0].bytes.as_slice())
    }
}

fn key_color(width: usize, i: usize) -> [u8; 3] {
    if (i / width / 32 + i % width / 32).is_multiple_of(2) {
        [41, 83, 129]
    } else {
        [89, 137, 173]
    }
}

fn delta_color(x: usize, y: usize) -> [u8; 3] {
    [(x * 13 + y * 17) as u8, (x * y) as u8, 211]
}

fn rgb32([r, g, b]: [u8; 3]) -> u32 {
    u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
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
}
