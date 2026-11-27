//! Compact absolute tile encodings. No external codec, heap scratch or unsafe code.
use super::{RAW, TILE_EDGE, rgb32};

pub(super) const PALETTE: u8 = 2;
pub(super) const RUNS: u8 = 3;
const MAX_PIXELS: usize = TILE_EDGE * TILE_EDGE;
const MAX_COLORS: usize = 16;

struct Palette {
    colors: [[u8; 3]; MAX_COLORS],
    indices: [u8; MAX_PIXELS],
    count: usize,
    pixels: usize,
}

fn bits(count: usize) -> usize {
    match count {
        2 => 1,
        3..=4 => 2,
        _ => 4,
    }
}

impl Palette {
    fn build(rgb: &[u8]) -> Option<Self> {
        let mut palette = Self {
            colors: [[0; 3]; MAX_COLORS],
            indices: [0; MAX_PIXELS],
            count: 0,
            pixels: rgb.len() / 3,
        };
        for (i, pixel) in rgb.chunks_exact(3).enumerate() {
            let color = [pixel[0], pixel[1], pixel[2]];
            let index = match palette.colors[..palette.count]
                .iter()
                .position(|entry| *entry == color)
            {
                Some(index) => index,
                None if palette.count < MAX_COLORS => {
                    let index = palette.count;
                    palette.colors[index] = color;
                    palette.count += 1;
                    index
                }
                None => return None,
            };
            palette.indices[i] = index as u8;
        }
        (palette.count >= 2).then_some(palette)
    }

    fn len(&self) -> usize {
        1 + self.count * 3 + (self.pixels * bits(self.count)).div_ceil(8)
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.push(self.count as u8);
        for color in &self.colors[..self.count] {
            out.extend_from_slice(color);
        }
        let width = bits(self.count);
        let per_byte = 8 / width;
        for group in self.indices[..self.pixels].chunks(per_byte) {
            let mut byte = 0;
            for (i, index) in group.iter().enumerate() {
                byte |= *index << (8 - width * (i + 1));
            }
            out.push(byte);
        }
    }
}

/// Each run is one length byte (length minus one) and three RGB bytes.
/// Runs may cross row boundaries; split after 256 pixels.
fn for_runs(rgb: &[u8], mut visit: impl FnMut(u8, &[u8]) -> bool) {
    let mut pixels = rgb.chunks_exact(3);
    let Some(mut color) = pixels.next() else {
        return;
    };
    let mut len_minus_one = 0u8;
    for pixel in pixels {
        if pixel == color && len_minus_one < u8::MAX {
            len_minus_one += 1;
        } else {
            if !visit(len_minus_one, color) {
                return;
            }
            color = pixel;
            len_minus_one = 0;
        }
    }
    let _ = visit(len_minus_one, color);
}

/// Append only the smallest encoding and only if the entire payload fits.
/// Input is one nonuniform, tightly packed tile from a validated grid.
pub(super) fn encode(rgb: &[u8], out: &mut Vec<u8>, budget: usize) -> Option<u8> {
    debug_assert!(!rgb.is_empty() && rgb.len() <= MAX_PIXELS * 3 && rgb.len().is_multiple_of(3));
    let palette = Palette::build(rgb);
    let mut mode = RAW;
    let mut size = rgb.len();
    if let Some(candidate) = &palette
        && candidate.len() < size
    {
        mode = PALETTE;
        size = candidate.len();
    }
    // Stop counting runs as soon as they cannot beat the best candidate.
    // In particular, alternating text pixels need not be scanned twice.
    let mut runs = 0usize;
    for_runs(rgb, |_, _| {
        runs += 1;
        let length = 2 + 4 * runs;
        length < size && length <= budget
    });
    let runs_len = 2 + 4 * runs;
    if runs_len < size && runs_len <= budget {
        mode = RUNS;
        size = runs_len;
    }
    if size > budget {
        return None;
    }
    match mode {
        RUNS => {
            out.extend_from_slice(&(runs as u16).to_be_bytes());
            for_runs(rgb, |len, color| {
                out.push(len);
                out.extend_from_slice(color);
                true
            });
        }
        PALETTE => palette.as_ref()?.write(out),
        _ => out.extend_from_slice(rgb),
    }
    Some(mode)
}

fn index_at(packed: &[u8], width: usize, pixel: usize) -> usize {
    let shift = 8 - width * (pixel % (8 / width) + 1);
    usize::from((packed[pixel / (8 / width)] >> shift) & ((1u8 << width) - 1))
}

/// Validate a prefix without allocating; returns its exact consumed length.
/// The caller validates the complete record before writing to the canvas.
pub(super) fn validate(mode: u8, data: &[u8], pixels: usize) -> Option<usize> {
    if !(1..=MAX_PIXELS).contains(&pixels) {
        return None;
    }
    match mode {
        PALETTE => {
            let count = usize::from(*data.first()?);
            if !(2..=MAX_COLORS).contains(&count) || count > pixels {
                return None;
            }
            let width = bits(count);
            let offset = 1 + count * 3;
            let len = offset + (pixels * width).div_ceil(8);
            let packed = data.get(offset..len)?;
            if (0..pixels).any(|i| index_at(packed, width, i) >= count) {
                return None;
            }
            let unused = (8 - (pixels * width) % 8) % 8;
            if unused != 0 && (*packed.last()? & ((1u8 << unused) - 1)) != 0 {
                return None;
            }
            Some(len)
        }
        RUNS => {
            let count = usize::from(u16::from_be_bytes([*data.first()?, *data.get(1)?]));
            if !(1..=pixels).contains(&count) {
                return None;
            }
            let len = 2 + count * 4;
            let runs = data.get(2..len)?;
            let mut covered = 0;
            for run in runs.chunks_exact(4) {
                covered += usize::from(run[0]) + 1;
                if covered > pixels {
                    return None;
                }
            }
            (covered == pixels).then_some(len)
        }
        _ => None,
    }
}

/// Decode a previously validated payload into its bounded contiguous tile.
/// No caller may use this function before whole-record validation succeeds.
pub(super) fn decode(mode: u8, data: &[u8], pixels: &mut [u32]) {
    match mode {
        PALETTE => {
            let count = usize::from(data[0]);
            let width = bits(count);
            let offset = 1 + count * 3;
            let packed = &data[offset..];
            let mut colors = [0; MAX_COLORS];
            for (target, rgb) in colors[..count]
                .iter_mut()
                .zip(data[1..offset].chunks_exact(3))
            {
                *target = rgb32(rgb);
            }
            for (i, target) in pixels.iter_mut().enumerate() {
                *target = colors[index_at(packed, width, i)];
            }
        }
        RUNS => {
            let mut offset = 0;
            for run in data[2..].chunks_exact(4) {
                let end = offset + usize::from(run[0]) + 1;
                pixels[offset..end].fill(rgb32(&run[1..4]));
                offset = end;
            }
        }
        _ => unreachable!("validated compact encoding"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_padding_and_invalid_index_are_rejected() {
        // Three pixels and three colors: 2-bit indices 0, 1, 2, then zero pad.
        let mut data = [3, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0b0001_1000];
        assert_eq!(validate(PALETTE, &data, 3), Some(data.len()));
        data[10] |= 1;
        assert_eq!(validate(PALETTE, &data, 3), None);
        data[10] = 0b0011_1000;
        assert_eq!(validate(PALETTE, &data, 3), None);
    }

    #[test]
    fn runs_must_cover_the_exact_tile() {
        let mut data = [0, 2, 255, 1, 2, 3, 0, 4, 5, 6];
        assert_eq!(validate(RUNS, &data, 257), Some(data.len()));
        assert_eq!(validate(RUNS, &data, 256), None);
        assert_eq!(validate(RUNS, &data, 258), None);
        data[1] = 3;
        assert_eq!(validate(RUNS, &data, 257), None);
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn early_run_budget_rejection_preserves_exact_best_encoding() {
        for rgb in [
            (0..MAX_PIXELS)
                .flat_map(|i| [(i * 31) as u8, (i / 256) as u8, 91])
                .collect::<Vec<_>>(),
            (0..MAX_PIXELS)
                .flat_map(|i| [(i % 16) as u8, 21, 29])
                .collect(),
            (0..MAX_PIXELS)
                .flat_map(|i| [(i / 128) as u8, 70, 80])
                .collect(),
        ] {
            let mut full = Vec::new();
            let mode = encode(&rgb, &mut full, MAX_PIXELS * 3).unwrap();
            for budget in [
                0,
                1,
                5,
                16,
                32,
                64,
                128,
                256,
                full.len().saturating_sub(1),
                full.len(),
                full.len() + 1,
            ] {
                let mut out = vec![99];
                let result = encode(&rgb, &mut out, budget);
                if budget < full.len() {
                    assert_eq!(result, None);
                    assert_eq!(out, [99]);
                } else {
                    assert_eq!(result, Some(mode));
                    assert_eq!(&out[1..], full.as_slice());
                }
            }
        }
    }
}
