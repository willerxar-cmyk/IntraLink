//! Platform-neutral, safe Rust, bounded absolute RGB tile updates.
//! Each delta depends on the preceding presentation state, not on JPEG bytes.
use crate::desktop::{MAX_FRAME_BYTES, VideoConfig};

mod compact;

pub const JPEG_TAG: u8 = 0;
pub const TILE_TAG: u8 = 1;
/// Full coverage, independent of the preceding presentation state.
pub const LOSSLESS_KEYFRAME_TAG: u8 = 2;
pub const TILE_EDGE: usize = 32;
pub const MAX_DELTA_BYTES: usize = 512 * 1024;
const HEADER: usize = 3;
const ENTRY: usize = 3;
const RAW: u8 = 0;
const SOLID: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaError {
    Mode,
    Dimensions,
    Malformed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delta {
    Unchanged,
    Encoded,
    KeyframeRequired,
}

#[derive(Clone, Copy)]
struct Grid {
    width: usize,
    height: usize,
    columns: usize,
    count: usize,
}

#[derive(Clone, Copy)]
struct Tile {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl Grid {
    fn new(config: VideoConfig) -> Self {
        let width = config.width() as usize;
        let height = config.height() as usize;
        let columns = width.div_ceil(TILE_EDGE);
        Self {
            width,
            height,
            columns,
            count: columns * height.div_ceil(TILE_EDGE),
        }
    }

    fn pixels(self) -> usize {
        self.width * self.height
    }

    fn tile(self, index: usize) -> Tile {
        let x = index % self.columns * TILE_EDGE;
        let y = index / self.columns * TILE_EDGE;
        Tile {
            x,
            y,
            width: TILE_EDGE.min(self.width - x),
            height: TILE_EDGE.min(self.height - y),
        }
    }

    fn row(self, rgb: &[u8], tile: Tile, row: usize) -> &[u8] {
        let start = ((tile.y + row) * self.width + tile.x) * 3;
        &rgb[start..start + tile.width * 3]
    }
}

/// Emit increasing tile indices. Each changed tile is absolute RGB, either
/// raw, solid, or negotiated compact palette/run data. Exceeding the budget requests an explicit
/// hybrid-mode JPEG keyframe; never return a partial delta.
pub fn encode_delta(
    config: VideoConfig,
    previous: &[u8],
    current: &[u8],
    output: &mut Vec<u8>,
    budget: usize,
) -> Result<Delta, DeltaError> {
    encode_tiles(config, Some(previous), current, output, budget)
}

/// Try an independent, exact RGB keyframe. A false result leaves an empty
/// output; the caller can use an explicitly negotiated alternative keyframe.
pub fn encode_keyframe(
    config: VideoConfig,
    current: &[u8],
    output: &mut Vec<u8>,
    budget: usize,
) -> Result<bool, DeltaError> {
    encode_tiles(config, None, current, output, budget).map(|result| result == Delta::Encoded)
}

fn encode_tiles(
    config: VideoConfig,
    previous: Option<&[u8]>,
    current: &[u8],
    output: &mut Vec<u8>,
    budget: usize,
) -> Result<Delta, DeltaError> {
    output.clear();
    let keyframe = previous.is_none();
    if !config.mode().supports_tiles() || (keyframe && !config.mode().supports_compact()) {
        return Err(DeltaError::Mode);
    }
    let grid = Grid::new(config);
    if current.len() != grid.pixels() * 3
        || previous.is_some_and(|frame| frame.len() != current.len())
    {
        return Err(DeltaError::Dimensions);
    }
    let budget = budget.min(MAX_DELTA_BYTES).min(MAX_FRAME_BYTES);
    if previous == Some(current) {
        return Ok(Delta::Unchanged);
    }
    // Each entry needs at least 6 bytes. Avoid scanning impossible keyframes.
    if budget < HEADER || (keyframe && budget < HEADER + grid.count * (ENTRY + 3)) {
        return Ok(Delta::KeyframeRequired);
    }
    let tag = if keyframe {
        LOSSLESS_KEYFRAME_TAG
    } else {
        TILE_TAG
    };
    output.extend_from_slice(&[tag, 0, 0]);
    let mut count = 0u16;
    let mut packed = [0u8; TILE_EDGE * TILE_EDGE * 3];
    for index in 0..grid.count {
        let tile = grid.tile(index);
        if previous.is_some_and(|frame| {
            (0..tile.height).all(|row| grid.row(frame, tile, row) == grid.row(current, tile, row))
        }) {
            continue;
        }
        // Test the first row's color, then compare entire subsequent rows.
        // Slice equality allows optimized byte comparisons on uniform regions
        // without rescanning RGB triples or allocating a reference row.
        let first_row = grid.row(current, tile, 0);
        let color = &first_row[..3];
        let solid = first_row.chunks_exact(3).all(|pixel| pixel == color)
            && (1..tile.height).all(|row| grid.row(current, tile, row) == first_row);
        if output.len() + ENTRY > budget {
            output.clear();
            return Ok(Delta::KeyframeRequired);
        }
        let mode_offset = output.len() + 2;
        output.extend_from_slice(&(index as u16).to_be_bytes());
        output.push(RAW);
        let remaining = budget - output.len();
        let mode = if solid && remaining >= 3 {
            output.extend_from_slice(color);
            Some(SOLID)
        } else if !solid && config.mode().supports_compact() {
            let row_bytes = tile.width * 3;
            let size = row_bytes * tile.height;
            for row in 0..tile.height {
                packed[row * row_bytes..(row + 1) * row_bytes]
                    .copy_from_slice(grid.row(current, tile, row));
            }
            compact::encode(&packed[..size], output, remaining)
        } else if !solid && tile.width * tile.height * 3 <= remaining {
            for row in 0..tile.height {
                output.extend_from_slice(grid.row(current, tile, row));
            }
            Some(RAW)
        } else {
            None
        };
        let Some(mode) = mode else {
            output.clear();
            return Ok(Delta::KeyframeRequired);
        };
        output[mode_offset] = mode;
        count += 1;
    }
    output[1..HEADER].copy_from_slice(&count.to_be_bytes());
    Ok(Delta::Encoded)
}

/// First validate every entry and the exact payload length; no allocation or
/// mutation occurs for malformed input. The second pass applies absolute tiles.
pub fn apply_delta(config: VideoConfig, data: &[u8], pixels: &mut [u32]) -> Result<(), DeltaError> {
    if !config.mode().supports_tiles() {
        return Err(DeltaError::Mode);
    }
    let grid = Grid::new(config);
    if pixels.len() != grid.pixels() {
        return Err(DeltaError::Dimensions);
    }
    walk(
        grid,
        config.mode().supports_compact(),
        TILE_TAG,
        data,
        |_, _, _| {},
    )?;
    apply_validated(
        grid,
        config.mode().supports_compact(),
        TILE_TAG,
        data,
        pixels,
    )
}

/// A keyframe must cover the complete output grid, including partial edge tiles.
/// Validate before resizing, so malformed data preserves even the old length.
pub fn apply_keyframe(
    config: VideoConfig,
    data: &[u8],
    pixels: &mut Vec<u32>,
) -> Result<(), DeltaError> {
    if !config.mode().supports_compact() {
        return Err(DeltaError::Mode);
    }
    let grid = Grid::new(config);
    walk(grid, true, LOSSLESS_KEYFRAME_TAG, data, |_, _, _| {})?;
    pixels.resize(grid.pixels(), 0);
    apply_validated(grid, true, LOSSLESS_KEYFRAME_TAG, data, pixels)
}

fn apply_validated(
    grid: Grid,
    compact_enabled: bool,
    tag: u8,
    data: &[u8],
    pixels: &mut [u32],
) -> Result<(), DeltaError> {
    let mut scratch = [0u32; TILE_EDGE * TILE_EDGE];
    walk(grid, compact_enabled, tag, data, |tile, mode, colors| {
        if matches!(mode, compact::PALETTE | compact::RUNS) {
            compact::decode(mode, colors, &mut scratch[..tile.width * tile.height]);
            for row in 0..tile.height {
                let start = (tile.y + row) * grid.width + tile.x;
                let offset = row * tile.width;
                pixels[start..start + tile.width]
                    .copy_from_slice(&scratch[offset..offset + tile.width]);
            }
            return;
        }
        for row in 0..tile.height {
            let start = (tile.y + row) * grid.width + tile.x;
            let target = &mut pixels[start..start + tile.width];
            if mode == SOLID {
                target.fill(rgb32(colors));
            } else {
                let offset = row * tile.width * 3;
                for (pixel, color) in target
                    .iter_mut()
                    .zip(colors[offset..offset + tile.width * 3].chunks_exact(3))
                {
                    *pixel = rgb32(color);
                }
            }
        }
    })
}

fn rgb32(color: &[u8]) -> u32 {
    (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2])
}

fn walk(
    grid: Grid,
    compact_enabled: bool,
    tag: u8,
    data: &[u8],
    mut visit: impl FnMut(Tile, u8, &[u8]),
) -> Result<(), DeltaError> {
    if data.len() < HEADER || data.len() > MAX_DELTA_BYTES || data[0] != tag {
        return Err(DeltaError::Malformed);
    }
    let count = usize::from(u16::from_be_bytes([data[1], data[2]]));
    if count == 0 || count > grid.count || (tag == LOSSLESS_KEYFRAME_TAG && count != grid.count) {
        return Err(DeltaError::Malformed);
    }
    let mut rest = &data[HEADER..];
    let mut previous = None;
    for _ in 0..count {
        if rest.len() < ENTRY {
            return Err(DeltaError::Malformed);
        }
        let index = usize::from(u16::from_be_bytes([rest[0], rest[1]]));
        let mode = rest[2];
        if index >= grid.count || previous.is_some_and(|last| index <= last) {
            return Err(DeltaError::Malformed);
        }
        previous = Some(index);
        let tile = grid.tile(index);
        rest = &rest[ENTRY..];
        let size = match mode {
            SOLID => 3,
            RAW => tile.width * tile.height * 3,
            compact::PALETTE | compact::RUNS if compact_enabled => {
                compact::validate(mode, rest, tile.width * tile.height)
                    .ok_or(DeltaError::Malformed)?
            }
            _ => return Err(DeltaError::Malformed),
        };
        if rest.len() < size {
            return Err(DeltaError::Malformed);
        }
        visit(tile, mode, &rest[..size]);
        rest = &rest[size..];
    }
    if !rest.is_empty() {
        return Err(DeltaError::Malformed);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
