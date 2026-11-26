//! Safe, single-pass RGBA-to-RGB letterboxing for a reusable output canvas.

/// Copy an already scaled source into a centered RGB canvas. All output bytes
/// are written once, including black bars. Reject bad geometry before mutation.
pub fn letterbox_rgba(
    source: &[u8],
    source_size: (u32, u32),
    target: &mut [u8],
    target_size: (u32, u32),
) -> bool {
    let (sw, sh) = (source_size.0 as usize, source_size.1 as usize);
    let (tw, th) = (target_size.0 as usize, target_size.1 as usize);
    if sw == 0 || sh == 0 || sw > tw || sh > th {
        return false;
    }
    let Some(source_stride) = sw.checked_mul(4) else {
        return false;
    };
    let Some(target_stride) = tw.checked_mul(3) else {
        return false;
    };
    if source_stride.checked_mul(sh) != Some(source.len())
        || target_stride.checked_mul(th) != Some(target.len())
    {
        return false;
    }
    let top = (th - sh) / 2;
    let left = (tw - sw) / 2 * 3;
    let content_end = left + sw * 3;
    let first = top * target_stride;
    let last = (top + sh) * target_stride;
    target[..first].fill(0);
    target[last..].fill(0);
    for (src, dst) in source
        .chunks_exact(source_stride)
        .zip(target[first..last].chunks_exact_mut(target_stride))
    {
        dst[..left].fill(0);
        dst[content_end..].fill(0);
        for (rgba, rgb) in src
            .chunks_exact(4)
            .zip(dst[left..content_end].chunks_exact_mut(3))
        {
            rgb.copy_from_slice(&rgba[..3]);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_size_overwrites_every_byte_and_ignores_alpha() {
        let mut canvas = [255; 6];
        assert!(letterbox_rgba(
            &[1, 2, 3, 0, 4, 5, 6, 255],
            (2, 1),
            &mut canvas,
            (2, 1)
        ));
        assert_eq!(canvas, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn odd_bars_are_centered_and_old_content_is_erased() {
        let mut canvas = [255; 5 * 4 * 3];
        let source = [7, 8, 9, 0].repeat(2 * 3);
        assert!(letterbox_rgba(&source, (2, 3), &mut canvas, (5, 4)));
        for (i, pixel) in canvas.chunks_exact(3).enumerate() {
            assert_eq!(
                pixel,
                if i / 5 < 3 && matches!(i % 5, 1 | 2) {
                    &[7, 8, 9]
                } else {
                    &[0, 0, 0]
                }
            );
        }
        let source = [1, 2, 3, 4].repeat(5);
        assert!(letterbox_rgba(&source, (5, 1), &mut canvas, (5, 4)));
        for (i, pixel) in canvas.chunks_exact(3).enumerate() {
            assert_eq!(pixel, if i / 5 == 1 { &[1, 2, 3] } else { &[0, 0, 0] });
        }
    }

    #[test]
    fn invalid_lengths_and_dimensions_leave_output_unchanged() {
        let mut canvas = [77; 12];
        let original = canvas;
        for (source, target) in [
            ((0, 1), (2, 2)),
            ((3, 1), (2, 2)),
            ((1, 1), (1, 1)),
            ((u32::MAX, u32::MAX), (u32::MAX, u32::MAX)),
        ] {
            assert!(!letterbox_rgba(&[1, 2, 3, 4], source, &mut canvas, target));
            assert_eq!(canvas, original);
        }
        assert!(!letterbox_rgba(&[0, 1, 2], (1, 1), &mut canvas, (2, 2)));
        assert_eq!(canvas, original);
    }
}
