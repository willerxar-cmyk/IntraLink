//! Bounded, allocation-free RGB24 -> NV12 conversion for future native encoders.
//! BT.709 limited-range, 8-bit Y and interleaved UV, 4:2:0 box chroma sampling.
//! This module is not yet connected to a hardware encoder or the wire protocol.

const MAX_WIDTH: usize = 3840;
const MAX_HEIGHT: usize = 2160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertError {
    Dimensions,
    Stride,
    Buffer,
}

/// An immutable, packed RGB plane; rows may include padding.
pub struct Rgb24<'a> {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub bytes: &'a [u8],
}

/// Caller-owned NV12 planes. Strides are in bytes; padding is not modified.
pub struct Nv12<'a> {
    pub y_stride: usize,
    pub uv_stride: usize,
    pub y: &'a mut [u8],
    pub uv: &'a mut [u8],
}

fn validate_plane(
    rows: usize,
    row_bytes: usize,
    stride: usize,
    len: usize,
) -> Result<(), ConvertError> {
    if stride < row_bytes {
        return Err(ConvertError::Stride);
    }
    let needed = (rows - 1)
        .checked_mul(stride)
        .and_then(|offset| offset.checked_add(row_bytes))
        .ok_or(ConvertError::Buffer)?;
    if len < needed {
        return Err(ConvertError::Buffer);
    }
    Ok(())
}

// Coefficients approximate BT.709 limited range with a 14-bit denominator.
// Chroma coefficients sum to zero, preserving neutral grayscale exactly.
const SCALE: i32 = 1 << 14;
const Y: [i32; 3] = [2991, 10064, 1016];
const U: [i32; 3] = [-1649, -5547, 7196];
const V: [i32; 3] = [7196, -6536, -660];

fn dot(rgb: [i32; 3], coefficients: [i32; 3]) -> i32 {
    rgb[0] * coefficients[0] + rgb[1] * coefficients[1] + rgb[2] * coefficients[2]
}

fn rgb(pixel: &[u8]) -> [i32; 3] {
    [
        i32::from(pixel[0]),
        i32::from(pixel[1]),
        i32::from(pixel[2]),
    ]
}

fn luma(color: [i32; 3]) -> u8 {
    (16 + ((dot(color, Y) + SCALE / 2) >> 14)).clamp(16, 235) as u8
}

fn chroma(sum: [i32; 3], coefficients: [i32; 3]) -> u8 {
    // Average the four source pixels before quantization, with one rounding.
    (128 + ((dot(sum, coefficients) + 2 * SCALE) >> 16)).clamp(16, 240) as u8
}

/// Convert one image, validating all planes before any output is written.
/// Only even dimensions are supported; no implicit crop or padding is performed.
/// The source and target cannot alias in safe Rust. Reuse output buffers across frames.
pub fn rgb24_to_nv12(source: Rgb24<'_>, target: Nv12<'_>) -> Result<(), ConvertError> {
    let (w, h) = (source.width, source.height);
    if !(2..=MAX_WIDTH).contains(&w)
        || !(2..=MAX_HEIGHT).contains(&h)
        || !w.is_multiple_of(2)
        || !h.is_multiple_of(2)
    {
        return Err(ConvertError::Dimensions);
    }
    let rgb_row = w * 3;
    validate_plane(h, rgb_row, source.stride, source.bytes.len())?;
    validate_plane(h, w, target.y_stride, target.y.len())?;
    validate_plane(h / 2, w, target.uv_stride, target.uv.len())?;

    // A single traversal of each 2x2 group computes four Y values and one UV pair.
    // No intermediate RGB copy, per-frame allocation, float arithmetic or unsafe code.
    for row in (0..h).step_by(2) {
        let src_top = row * source.stride;
        let src_bottom = src_top + source.stride;
        let y_top = row * target.y_stride;
        let y_bottom = y_top + target.y_stride;
        let uv_row = row / 2 * target.uv_stride;
        let top = &source.bytes[src_top..src_top + rgb_row];
        let bottom = &source.bytes[src_bottom..src_bottom + rgb_row];
        for x in (0..w).step_by(2) {
            let a = rgb(&top[x * 3..]);
            let b = rgb(&top[(x + 1) * 3..]);
            let c = rgb(&bottom[x * 3..]);
            let d = rgb(&bottom[(x + 1) * 3..]);
            target.y[y_top + x] = luma(a);
            target.y[y_top + x + 1] = luma(b);
            target.y[y_bottom + x] = luma(c);
            target.y[y_bottom + x + 1] = luma(d);
            let sum = std::array::from_fn(|i| a[i] + b[i] + c[i] + d[i]);
            target.uv[uv_row + x] = chroma(sum, U);
            target.uv[uv_row + x + 1] = chroma(sum, V);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert_uniform(color: [u8; 3]) -> ([u8; 4], [u8; 2]) {
        let bytes = color.repeat(4);
        let mut y = [0; 4];
        let mut uv = [0; 2];
        rgb24_to_nv12(
            Rgb24 {
                width: 2,
                height: 2,
                stride: 6,
                bytes: &bytes,
            },
            Nv12 {
                y_stride: 2,
                uv_stride: 2,
                y: &mut y,
                uv: &mut uv,
            },
        )
        .unwrap();
        (y, uv)
    }

    #[test]
    fn black_white_and_neutral_grays() {
        assert_eq!(convert_uniform([0; 3]), ([16; 4], [128; 2]));
        assert_eq!(convert_uniform([255; 3]), ([235; 4], [128; 2]));
        for level in 0..=255 {
            let (y, uv) = convert_uniform([level; 3]);
            assert!((16..=235).contains(&y[0]));
            assert_eq!(uv, [128; 2]);
        }
    }

    #[test]
    fn primary_color_vectors() {
        assert_eq!(convert_uniform([255, 0, 0]), ([63; 4], [102, 240]));
        assert_eq!(convert_uniform([0, 255, 0]), ([173; 4], [42, 26]));
        assert_eq!(convert_uniform([0, 0, 255]), ([32; 4], [240, 118]));
    }

    #[test]
    fn averages_chroma_and_preserves_stride_padding() {
        let source = [255, 0, 0, 0, 255, 0, 99, 0, 0, 255, 255, 255, 255];
        let mut y = [77; 6];
        let mut uv = [77; 4];
        rgb24_to_nv12(
            Rgb24 {
                width: 2,
                height: 2,
                stride: 7,
                bytes: &source,
            },
            Nv12 {
                y_stride: 3,
                uv_stride: 4,
                y: &mut y,
                uv: &mut uv,
            },
        )
        .unwrap();
        assert_eq!(y, [63, 173, 77, 32, 235, 77]);
        assert_eq!(uv, [128, 128, 77, 77]);
    }

    #[test]
    fn invalid_layouts_are_atomic() {
        let source = [0; 12];
        for (width, height, stride, y_stride, uv_stride, uv_len, expected) in [
            (1, 2, 6, 2, 2, 2, ConvertError::Dimensions),
            (2, 3, 6, 2, 2, 2, ConvertError::Dimensions),
            (3842, 2, 6, 2, 2, 2, ConvertError::Dimensions),
            (2, 2, 5, 2, 2, 2, ConvertError::Stride),
            (2, 2, 6, 1, 2, 2, ConvertError::Stride),
            (2, 2, 6, 2, 1, 2, ConvertError::Stride),
            (2, 2, usize::MAX, 2, 2, 2, ConvertError::Buffer),
            (2, 2, 6, 2, 2, 1, ConvertError::Buffer),
        ] {
            let mut y = [77; 4];
            let mut uv = [77; 2];
            let result = rgb24_to_nv12(
                Rgb24 {
                    width,
                    height,
                    stride,
                    bytes: &source,
                },
                Nv12 {
                    y_stride,
                    uv_stride,
                    y: &mut y,
                    uv: &mut uv[..uv_len],
                },
            );
            assert_eq!(result, Err(expected));
            assert_eq!(y, [77; 4]);
            assert_eq!(uv, [77; 2]);
        }
    }
}
