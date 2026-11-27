//! Full-canvas comparison against the actual captured RGB reference.
//! Used only with locally generated diagnostic content, never user pixels.
use anyhow::{Result, ensure};
use intralink_core::{desktop::VideoConfig, media::LOSSLESS_KEYFRAME_TAG};

const BLOCK: usize = 32;
// Diagnostic tolerances for the calm scene at quality 75; not a guarantee
// about arbitrary lossy JPEG content. Lossless keyframes require equality.
const MAX_MEAN: f64 = 6.0;
const MAX_BLOCK_MEAN: f64 = 24.0;
const MAX_LARGE_ERROR_FRACTION: f64 = 0.01;

pub(super) struct Reference {
    pub(super) sequence: usize,
    pub(super) tag: u8,
    pub(super) rgb: Vec<u8>,
}

#[derive(Debug, Default)]
pub(super) struct Fidelity {
    pub(super) frames: usize,
    pub(super) pixels: u64,
    pub(super) max_channel_mae: f64,
    pub(super) max_block_mae: f64,
    pub(super) max_large_fraction: f64,
    pub(super) lossless_frames: usize,
}

impl Fidelity {
    pub(super) fn check(
        &mut self,
        config: VideoConfig,
        reference: &Reference,
        pixels: &[u32],
    ) -> Result<()> {
        let (w, h) = (config.width() as usize, config.height() as usize);
        ensure!(
            pixels.len() == w * h && reference.rgb.len() == w * h * 3,
            "Visual reference geometry mismatch"
        );
        let mut errors = [0u64; 3];
        let mut block_errors = vec![0u64; w.div_ceil(BLOCK) * h.div_ceil(BLOCK)];
        let mut large = 0u64;
        for (i, (rgb, decoded)) in reference.rgb.chunks_exact(3).zip(pixels).enumerate() {
            let mut sum = 0u64;
            let mut worst = 0u8;
            for (channel, shift) in [16, 8, 0].into_iter().enumerate() {
                let error = rgb[channel].abs_diff((decoded >> shift) as u8);
                errors[channel] += u64::from(error);
                sum += u64::from(error);
                worst = worst.max(error);
            }
            block_errors[(i / w / BLOCK) * w.div_ceil(BLOCK) + (i % w / BLOCK)] += sum;
            large += u64::from(worst > 48);
        }
        let mae = errors
            .into_iter()
            .map(|e| e as f64 / (w * h) as f64)
            .fold(0.0, f64::max);
        let block_mae = block_errors
            .into_iter()
            .enumerate()
            .map(|(i, error)| {
                let x = (i % w.div_ceil(BLOCK)) * BLOCK;
                let y = (i / w.div_ceil(BLOCK)) * BLOCK;
                error as f64 / ((w - x).min(BLOCK) * (h - y).min(BLOCK) * 3) as f64
            })
            .fold(0.0, f64::max);
        let large_fraction = large as f64 / (w * h) as f64;
        if reference.tag == LOSSLESS_KEYFRAME_TAG {
            ensure!(
                mae == 0.0,
                "Lossless visual frame differs at sequence {}",
                reference.sequence
            );
        } else {
            ensure!(
                mae <= MAX_MEAN
                    && block_mae <= MAX_BLOCK_MEAN
                    && large_fraction <= MAX_LARGE_ERROR_FRACTION,
                "Visual frame corrupted or outside diagnostic tolerance at sequence {}: channel_mae={mae:.3}, block_mae={block_mae:.3}, large_fraction={large_fraction:.5}",
                reference.sequence
            );
        }
        self.frames += 1;
        self.pixels += (w * h) as u64;
        self.lossless_frames += usize::from(reference.tag == LOSSLESS_KEYFRAME_TAG);
        self.max_channel_mae = self.max_channel_mae.max(mae);
        self.max_block_mae = self.max_block_mae.max(block_mae);
        self.max_large_fraction = self.max_large_fraction.max(large_fraction);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use intralink_core::media::{JPEG_TAG, TILE_TAG};

    #[test]
    fn detects_channel_swap_local_corruption_and_lossless_mismatch() -> Result<()> {
        let config = VideoConfig::new(540, 60, 75, 30).unwrap();
        let (w, h) = (config.width() as usize, config.height() as usize);
        let mut pixels = vec![0; w * h];
        super::super::visual_fixture::draw(&mut pixels, w, h, 21);
        let mut reference = Reference {
            sequence: 1,
            tag: LOSSLESS_KEYFRAME_TAG,
            rgb: pixels
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect(),
        };
        let mut stats = Fidelity::default();
        stats.check(config, &reference, &pixels)?;
        pixels[0] ^= 1;
        assert!(stats.check(config, &reference, &pixels).is_err());
        pixels[0] ^= 1;
        reference.tag = JPEG_TAG;
        let swapped: Vec<_> = pixels
            .iter()
            .map(|p| ((p & 255) << 16) | (p & 0xff00) | ((p >> 16) & 255))
            .collect();
        assert!(stats.check(config, &reference, &swapped).is_err());
        reference.tag = TILE_TAG;
        for row in pixels.chunks_exact_mut(w).skip(320).take(32) {
            row[416..448].fill(0);
        }
        assert!(stats.check(config, &reference, &pixels).is_err());
        assert_eq!(stats.frames, 1);
        assert_eq!(stats.lossless_frames, 1);
        Ok(())
    }

    #[test]
    fn accepts_real_jpeg_and_ordered_tiles_for_generated_scene() -> Result<()> {
        use crate::desktop::{
            codec::{Decoder, Encoder},
            visual_fixture,
        };
        use intralink_core::desktop::CodecMode;
        use intralink_transport::desktop::Capture;
        let config = VideoConfig::new(540, 60, 75, 30)
            .unwrap()
            .with_mode(CodecMode::Compact);
        let (w, h) = (config.width() as usize, config.height() as usize);
        let mut original = vec![0; w * h];
        let mut encoder = Encoder::new(config);
        let mut decoder = Decoder::default();
        let mut buffer = Vec::new();
        let mut decoded = Vec::new();
        let mut stats = Fidelity::default();
        for phase in [0, 17, 34, 51] {
            visual_fixture::draw(&mut original, w, h, phase);
            let rgb: Vec<u8> = original
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect();
            let rgba = original
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255])
                .collect();
            let image = image::RgbaImage::from_raw(w as u32, h as u32, rgba).unwrap();
            let Capture::Frame(data) = encoder.encode(&image, buffer)? else {
                anyhow::bail!("Missing changed visual record")
            };
            decoder.decode_into(config, &data, &mut decoded)?;
            stats.check(
                config,
                &Reference {
                    sequence: phase + 1,
                    tag: data[0],
                    rgb,
                },
                &decoded,
            )?;
            visual_fixture::verify_decoded(config, &decoded)?;
            buffer = data;
        }
        assert_eq!(stats.frames, 4);
        Ok(())
    }
}
