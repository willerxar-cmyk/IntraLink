//! Aspect-preserving CPU resize with reusable buffers and native SIMD dispatch.
use anyhow::{Result, ensure};
use fast_image_resize::{
    FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer,
    images::{Image, ImageRef},
};
use image::RgbaImage;

use super::codec::validate_source_size;
use intralink_core::canvas::letterbox_rgba;

#[derive(Default)]
pub(super) struct CanvasResizer {
    resizer: Resizer,
    scaled: Vec<u8>,
}

pub(super) fn fitted(source: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = (u64::from(source.0), u64::from(source.1));
    let (tw, th) = (u64::from(target.0), u64::from(target.1));
    if sw * th >= sh * tw {
        (target.0, ((sh * tw + sw / 2) / sw).clamp(1, th) as u32)
    } else {
        (((sw * th + sh / 2) / sh).clamp(1, tw) as u32, target.1)
    }
}

impl CanvasResizer {
    pub(super) fn fill_rgb(
        &mut self,
        frame: &RgbaImage,
        target: &mut [u8],
        target_size: (u32, u32),
    ) -> Result<()> {
        let source_size = frame.dimensions();
        validate_source_size(source_size.0, source_size.1)?;
        validate_source_size(target_size.0, target_size.1)?;
        let expected = u64::from(target_size.0) * u64::from(target_size.1) * 3;
        ensure!(target.len() as u64 == expected, "Invalid RGB canvas size");
        let scaled_size = fitted(source_size, target_size);
        // No resize, scratch allocation or intermediate copy at matching size.
        let bytes = if source_size == scaled_size {
            frame.as_raw().as_slice()
        } else {
            let length = scaled_size.0 as usize * scaled_size.1 as usize * 4;
            self.scaled.resize(length, 0);
            let source = ImageRef::new(
                source_size.0,
                source_size.1,
                frame.as_raw(),
                PixelType::U8x4,
            )?;
            let mut destination = Image::from_slice_u8(
                scaled_size.0,
                scaled_size.1,
                &mut self.scaled,
                PixelType::U8x4,
            )?;
            // Screen capture is opaque RGB; the fourth channel is not opacity.
            // Keep antialiasing for downsampling and avoid alpha premultiplication.
            let options = ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear))
                .use_alpha(false);
            self.resizer.resize(&source, &mut destination, &options)?;
            self.scaled.as_slice()
        };
        ensure!(
            letterbox_rgba(bytes, scaled_size, target, target_size),
            "Invalid scaled image or RGB canvas geometry"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn aspect_and_rounding() {
        assert_eq!(fitted((3321, 1869), (1920, 1080)), (1919, 1080));
        assert_eq!(fitted((1, 5000), (960, 540)), (1, 540));
        assert_eq!(fitted((5000, 1), (960, 540)), (960, 1));
        assert_eq!(fitted((4, 3), (960, 540)), (720, 540));
    }

    #[test]
    fn opaque_screen_colors_and_reused_buffer() -> Result<()> {
        let mut scaler = CanvasResizer::default();
        let source = RgbaImage::from_pixel(8, 8, Rgba([21, 75, 193, 0]));
        let mut target = vec![255; 8 * 4 * 3];
        scaler.fill_rgb(&source, &mut target, (8, 4))?;
        for (i, pixel) in target.chunks_exact(3).enumerate() {
            assert_eq!(
                pixel,
                if (2..6).contains(&(i % 8)) {
                    &[21, 75, 193]
                } else {
                    &[0, 0, 0]
                }
            );
        }
        let ptr = scaler.scaled.as_ptr();
        scaler.fill_rgb(&source, &mut target, (8, 4))?;
        assert_eq!(scaler.scaled.as_ptr(), ptr);
        Ok(())
    }

    #[test]
    fn downsampling_preserves_antialiasing() -> Result<()> {
        let mut scaler = CanvasResizer::default();
        let source = RgbaImage::from_fn(32, 32, |x, y| {
            let v = if (x + y) % 2 == 0 { 0 } else { 255 };
            Rgba([v, v, v, 255])
        });
        let mut output = vec![0; 8 * 8 * 3];
        scaler.fill_rgb(&source, &mut output, (8, 8))?;
        assert!(output.iter().all(|value| (110..=145).contains(value)));
        let reference =
            image::imageops::resize(&source, 8, 8, image::imageops::FilterType::Triangle);
        for (actual, expected) in output.chunks_exact(3).zip(reference.pixels()) {
            assert!(
                actual
                    .iter()
                    .zip(expected.0)
                    .all(|(a, b)| a.abs_diff(b) <= 2)
            );
        }
        Ok(())
    }

    #[test]
    fn invalid_geometry_is_atomic_and_exact_size_skips_resize() -> Result<()> {
        let mut scaler = CanvasResizer::default();
        let source = RgbaImage::from_pixel(4, 2, Rgba([1, 2, 3, 0]));
        let mut target = vec![99; 24];
        assert!(scaler.fill_rgb(&source, &mut target, (0, 8)).is_err());
        assert!(scaler.fill_rgb(&source, &mut target, (8, 8)).is_err());
        assert_eq!(target, vec![99; 24]);
        scaler.fill_rgb(&source, &mut target, (4, 2))?;
        assert_eq!(target, [1, 2, 3].repeat(8));
        assert!(scaler.scaled.is_empty());
        Ok(())
    }
}
