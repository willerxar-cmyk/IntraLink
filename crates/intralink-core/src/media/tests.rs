use super::*;

fn config() -> VideoConfig {
    VideoConfig::new(540, 30, 75, 30)
        .unwrap()
        .with_mode(crate::desktop::CodecMode::Tiles)
}

#[test]
fn edge_tiles_round_trip_and_solid_tiles_are_nine_bytes() {
    let config = config();
    let grid = Grid::new(config);
    let before = vec![0; grid.pixels() * 3];
    let mut after = before.clone();
    let tile = grid.tile(grid.count - 1);
    for row in tile.y..tile.y + tile.height {
        for x in tile.x..tile.x + tile.width {
            let offset = (row * grid.width + x) * 3;
            after[offset..offset + 3].copy_from_slice(&[1, 2, 3]);
        }
    }
    let mut data = Vec::new();
    assert_eq!(
        encode_delta(config, &before, &after, &mut data, MAX_DELTA_BYTES),
        Ok(Delta::Encoded)
    );
    assert_eq!(data.len(), 9);
    let mut pixels = vec![0; grid.pixels()];
    apply_delta(config, &data, &mut pixels).unwrap();
    assert!(
        pixels
            .iter()
            .zip(after.chunks_exact(3))
            .all(|(pixel, color)| *pixel == rgb32(color))
    );
}

#[test]
fn ordered_deltas_reconstruct_changed_tiles_and_restore_pixels() {
    let config = config();
    let grid = Grid::new(config);
    let mut before = vec![0; grid.pixels() * 3];
    let mut pixels = vec![0; grid.pixels()];
    let mut data = Vec::new();
    for offset in [0, 3, (grid.pixels() - 1) * 3, 0] {
        let mut after = before.clone();
        after[offset] ^= 255;
        assert_eq!(
            encode_delta(config, &before, &after, &mut data, MAX_DELTA_BYTES),
            Ok(Delta::Encoded)
        );
        apply_delta(config, &data, &mut pixels).unwrap();
        assert!(
            pixels
                .iter()
                .zip(after.chunks_exact(3))
                .all(|(pixel, color)| *pixel == rgb32(color))
        );
        before = after;
    }
    assert_eq!(
        encode_delta(config, &before, &before, &mut data, 0),
        Ok(Delta::Unchanged)
    );
    assert!(data.is_empty());
}

#[test]
fn budget_exhaustion_never_emits_partial_updates() {
    let config = config();
    let grid = Grid::new(config);
    let before = vec![0; grid.pixels() * 3];
    let mut after = before.clone();
    after[0] = 1;
    let mut data = vec![123];
    assert_eq!(
        encode_delta(config, &before, &after, &mut data, 9),
        Ok(Delta::KeyframeRequired)
    );
    assert!(data.is_empty());
    assert_eq!(
        encode_delta(config, &before[..1], &after, &mut data, MAX_DELTA_BYTES),
        Err(DeltaError::Dimensions)
    );
}

#[test]
fn malformed_updates_do_not_mutate_pixels() {
    let config = config();
    let grid = Grid::new(config);
    let mut pixels = vec![0x112233; grid.pixels()];
    let one = [TILE_TAG, 0, 1, 0, 0, SOLID, 1, 2, 3];
    for len in 0..one.len() {
        assert!(apply_delta(config, &one[..len], &mut pixels).is_err());
    }
    for invalid in [
        vec![TILE_TAG, 0, 0],
        vec![TILE_TAG, 0, 1, 255, 255, SOLID, 1, 2, 3],
        vec![TILE_TAG, 0, 1, 0, 0, 2, 1, 2, 3],
        vec![TILE_TAG, 0, 2, 0, 0, SOLID, 1, 2, 3, 0, 0, SOLID, 4, 5, 6],
        vec![TILE_TAG, 0, 2, 0, 1, SOLID, 1, 2, 3, 0, 0, SOLID, 4, 5, 6],
        [one.as_slice(), &[1]].concat(),
        vec![TILE_TAG; MAX_DELTA_BYTES + 1],
    ] {
        assert!(apply_delta(config, &invalid, &mut pixels).is_err());
    }
    assert!(pixels.iter().all(|pixel| *pixel == 0x112233));
    assert!(apply_delta(config, &one, &mut pixels[..1]).is_err());
}

fn compact_config() -> VideoConfig {
    config().with_mode(crate::desktop::CodecMode::Compact)
}

/// A full 32x32 tile, with all other tiles unchanged.
fn fixture(config: VideoConfig, mut pattern: impl FnMut(usize) -> [u8; 3]) -> (Vec<u8>, Vec<u8>) {
    let grid = Grid::new(config);
    let before = vec![0; grid.pixels() * 3];
    let mut after = before.clone();
    for y in 0..TILE_EDGE {
        for x in 0..TILE_EDGE {
            let offset = (y * grid.width + x) * 3;
            after[offset..offset + 3].copy_from_slice(&pattern(y * TILE_EDGE + x));
        }
    }
    (before, after)
}

fn assert_pixels(after: &[u8], pixels: &[u32]) {
    assert!(
        pixels
            .iter()
            .zip(after.chunks_exact(3))
            .all(|(pixel, color)| *pixel == rgb32(color))
    );
}

#[test]
fn compact_selects_smallest_exact_tile_representation() {
    let config = compact_config();
    let grid = Grid::new(config);
    let cases = [
        (
            fixture(
                config,
                |i| if i % 2 == 0 { [20, 40, 60] } else { [0, 0, 0] },
            ),
            compact::PALETTE,
            141,
        ),
        (
            fixture(config, |i| if i < 512 { [20, 40, 60] } else { [0, 0, 0] }),
            compact::RUNS,
            24,
        ),
        (
            fixture(config, |i| [(i % 251) as u8, (i / 251) as u8, 40]),
            RAW,
            3078,
        ),
        (fixture(config, |_| [20, 40, 60]), SOLID, 9),
    ];
    for ((before, after), mode, len) in cases {
        let mut data = Vec::new();
        let mut pixels = vec![0; grid.pixels()];
        assert_eq!(
            encode_delta(config, &before, &after, &mut data, len),
            Ok(Delta::Encoded)
        );
        assert_eq!(data[5], mode);
        assert_eq!(data.len(), len);
        apply_delta(config, &data, &mut pixels).unwrap();
        assert_pixels(&after, &pixels);
        assert_eq!(
            encode_delta(config, &before, &after, &mut data, len - 1),
            Ok(Delta::KeyframeRequired)
        );
        assert!(data.is_empty());
    }
}

#[test]
fn compact_records_are_rejected_by_basic_tiles_and_jpeg() {
    use crate::desktop::CodecMode;
    let config = compact_config();
    let (before, after) = fixture(config, |i| [(i % 2) as u8, 1, 0]);
    let mut data = Vec::new();
    assert_eq!(
        encode_delta(config, &before, &after, &mut data, MAX_DELTA_BYTES),
        Ok(Delta::Encoded)
    );
    assert_eq!(data[5], compact::PALETTE);
    let mut pixels = vec![0x123456; Grid::new(config).pixels()];
    for mode in [CodecMode::Tiles, CodecMode::Jpeg] {
        assert!(apply_delta(config.with_mode(mode), &data, &mut pixels).is_err());
        assert!(pixels.iter().all(|pixel| *pixel == 0x123456));
    }
}

#[test]
fn malformed_later_compact_tile_cannot_apply_earlier_tile() {
    let config = compact_config();
    let mut pixels = vec![0x123456; Grid::new(config).pixels()];
    // First entry is valid. The second promises two runs but only contains one.
    let data = [
        TILE_TAG,
        0,
        2,
        0,
        0,
        SOLID,
        1,
        2,
        3,
        0,
        1,
        compact::RUNS,
        0,
        2,
        255,
        4,
        5,
        6,
    ];
    assert!(apply_delta(config, &data, &mut pixels).is_err());
    assert!(pixels.iter().all(|pixel| *pixel == 0x123456));
}

#[test]
fn compact_palette_cardinalities_and_edge_geometry_round_trip() {
    let config = compact_config();
    let grid = Grid::new(config);
    let tile = grid.tile(grid.count - 1);
    let mut before = vec![0; grid.pixels() * 3];
    let mut pixels = vec![0; grid.pixels()];
    let mut data = Vec::new();
    // Includes both index-width boundaries and a >16-color raw case.
    for colors in [2, 3, 4, 5, 16, 17, 2] {
        let mut after = before.clone();
        for y in 0..tile.height {
            for x in 0..tile.width {
                let offset = ((tile.y + y) * grid.width + tile.x + x) * 3;
                let color = ((y * tile.width + x) % colors) as u8;
                after[offset..offset + 3].copy_from_slice(&[color, 200, 30]);
            }
        }
        assert_eq!(
            encode_delta(config, &before, &after, &mut data, MAX_DELTA_BYTES),
            Ok(Delta::Encoded)
        );
        assert_eq!(data[5], if colors <= 16 { compact::PALETTE } else { RAW });
        apply_delta(config, &data, &mut pixels).unwrap();
        assert_pixels(&after, &pixels);
        before = after;
    }
}

#[test]
fn compact_never_exceeds_basic_payload_under_same_budget() {
    use crate::desktop::CodecMode;
    let config = compact_config();
    let mut state = 0x1234_5678u32;
    for colors in [2u32, 4, 16, 64, 251] {
        let (before, after) = fixture(config, |_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            [((state >> 16) % colors) as u8, 73, 199]
        });
        let mut basic = Vec::new();
        let mut compact = Vec::new();
        assert_eq!(
            encode_delta(
                config.with_mode(CodecMode::Tiles),
                &before,
                &after,
                &mut basic,
                MAX_DELTA_BYTES
            ),
            Ok(Delta::Encoded)
        );
        assert_eq!(
            encode_delta(config, &before, &after, &mut compact, MAX_DELTA_BYTES),
            Ok(Delta::Encoded)
        );
        assert!(compact.len() <= basic.len());
        let mut pixels = vec![0; Grid::new(config).pixels()];
        apply_delta(config, &compact, &mut pixels).unwrap();
        assert_pixels(&after, &pixels);
    }
}

#[test]
fn lossless_uniform_keyframes_have_full_coverage_at_all_resolutions() {
    use crate::desktop::CodecMode;
    for (height, expected_count) in [(540, 510), (720, 920), (1080, 2040)] {
        let config = VideoConfig::new(height, 30, 75, 30)
            .unwrap()
            .with_mode(CodecMode::Compact);
        let grid = Grid::new(config);
        assert_eq!(grid.count, expected_count);
        let rgb = [17, 43, 199].repeat(grid.pixels());
        let mut encoded = Vec::new();
        let expected_size = HEADER + expected_count * (ENTRY + 3);
        assert_eq!(
            encode_keyframe(config, &rgb, &mut encoded, expected_size),
            Ok(true)
        );
        assert_eq!(encoded[0], LOSSLESS_KEYFRAME_TAG);
        assert_eq!(encoded.len(), expected_size);
        let mut pixels = vec![0xdeadbe; 11];
        apply_keyframe(config, &encoded, &mut pixels).unwrap();
        assert_eq!(pixels.len(), grid.pixels());
        assert_pixels(&rgb, &pixels);
        assert_eq!(
            encode_keyframe(config, &rgb, &mut encoded, expected_size - 1),
            Ok(false)
        );
        assert!(encoded.is_empty());
    }
}

#[test]
fn lossless_keyframe_and_following_delta_preserve_every_pixel() {
    let config = compact_config();
    let grid = Grid::new(config);
    let (_, mut frame) = fixture(config, |i| {
        if i % 2 == 0 {
            [3, 7, 11]
        } else {
            [200, 90, 12]
        }
    });
    let last = frame.len() - 3;
    frame[last..].copy_from_slice(&[9, 8, 7]);
    let mut encoded = Vec::new();
    assert_eq!(
        encode_keyframe(config, &frame, &mut encoded, MAX_DELTA_BYTES),
        Ok(true)
    );
    let mut pixels = Vec::new();
    apply_keyframe(config, &encoded, &mut pixels).unwrap();
    assert_pixels(&frame, &pixels);
    let old = frame.clone();
    frame[last..].copy_from_slice(&[6, 5, 4]);
    assert_eq!(
        encode_delta(config, &old, &frame, &mut encoded, MAX_DELTA_BYTES),
        Ok(Delta::Encoded)
    );
    apply_delta(config, &encoded, &mut pixels).unwrap();
    assert_eq!(pixels.len(), grid.pixels());
    assert_pixels(&frame, &pixels);
}

#[test]
fn invalid_keyframes_never_resize_or_mutate_existing_canvas() {
    let config = compact_config();
    let grid = Grid::new(config);
    let mut valid = Vec::new();
    assert_eq!(
        encode_keyframe(
            config,
            &vec![8; grid.pixels() * 3],
            &mut valid,
            MAX_DELTA_BYTES
        ),
        Ok(true)
    );
    let mut duplicate = valid.clone();
    // Last tile index is duplicated; prior entries are all valid.
    let last = duplicate.len() - (ENTRY + 3);
    duplicate[last..last + 2].copy_from_slice(&((grid.count - 2) as u16).to_be_bytes());
    let mut missing = valid[..valid.len() - (ENTRY + 3)].to_vec();
    missing[1..HEADER].copy_from_slice(&((grid.count - 1) as u16).to_be_bytes());
    let mut wrong_tag = valid.clone();
    wrong_tag[0] = TILE_TAG;
    for invalid in [
        duplicate,
        missing,
        wrong_tag,
        valid[..valid.len() - 1].to_vec(),
        [valid.as_slice(), &[0]].concat(),
    ] {
        let mut pixels = vec![0xabcdef; 7];
        assert!(apply_keyframe(config, &invalid, &mut pixels).is_err());
        assert_eq!(pixels, vec![0xabcdef; 7]);
    }
    let mut full = vec![0xabcdef; grid.pixels()];
    assert!(apply_delta(config, &valid, &mut full).is_err());
    assert!(full.iter().all(|pixel| *pixel == 0xabcdef));
}

#[test]
fn keyframe_budget_and_mode_fail_closed() {
    use crate::desktop::CodecMode;
    let config = compact_config();
    let grid = Grid::new(config);
    let mut frame = vec![0; grid.pixels() * 3];
    let mut state = 0x1234_5678u32;
    for byte in &mut frame {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        *byte = (state >> 24) as u8;
    }
    let mut encoded = vec![0xff];
    assert_eq!(
        encode_keyframe(config, &frame, &mut encoded, usize::MAX),
        Ok(false)
    );
    assert!(encoded.is_empty());
    frame.fill(8);
    assert_eq!(
        encode_keyframe(config, &frame, &mut encoded, MAX_DELTA_BYTES),
        Ok(true)
    );
    let mut pixels = vec![0xabcdef];
    for mode in [CodecMode::Jpeg, CodecMode::Tiles] {
        let mode_config = config.with_mode(mode);
        assert_eq!(
            apply_keyframe(mode_config, &encoded, &mut pixels),
            Err(DeltaError::Mode)
        );
        assert_eq!(pixels, vec![0xabcdef]);
        assert_eq!(
            encode_keyframe(mode_config, &frame, &mut Vec::new(), MAX_DELTA_BYTES),
            Err(DeltaError::Mode)
        );
    }
    assert_eq!(
        encode_keyframe(config, &frame[..1], &mut encoded, MAX_DELTA_BYTES),
        Err(DeltaError::Dimensions)
    );
    assert!(encoded.is_empty());
}

#[test]
fn uniform_row_comparison_preserves_nonuniform_edge_tiles() {
    let config = config();
    let grid = Grid::new(config);
    let before = vec![0; grid.pixels() * 3];
    let tile = grid.tile(grid.count - 1);
    assert!(tile.height < TILE_EDGE && tile.height > 1);
    let mut after = before.clone();
    for y in tile.y..tile.y + tile.height {
        for x in tile.x..tile.x + tile.width {
            let offset = (y * grid.width + x) * 3;
            after[offset..offset + 3].copy_from_slice(&[15, 20, 25]);
        }
    }
    // The first row is uniform but the last row has a different color.
    let last_row = (tile.y + tile.height - 1) * grid.width + tile.x;
    for pixel in after[last_row * 3..(last_row + tile.width) * 3].chunks_exact_mut(3) {
        pixel.copy_from_slice(&[30, 40, 50]);
    }
    let mut data = Vec::new();
    assert_eq!(
        encode_delta(config, &before, &after, &mut data, MAX_DELTA_BYTES),
        Ok(Delta::Encoded)
    );
    assert_eq!(data[5], RAW);
    let mut pixels = vec![0; grid.pixels()];
    apply_delta(config, &data, &mut pixels).unwrap();
    assert_pixels(&after, &pixels);
}

#[test]
fn failed_compact_delta_implies_no_complete_keyframe_with_same_or_smaller_budget() {
    let config = VideoConfig::new(540, 60, 75, 30)
        .unwrap()
        .with_mode(crate::desktop::CodecMode::Compact);
    let grid = Grid::new(config);
    let mut previous = vec![0; grid.pixels() * 3];
    let mut current = previous.clone();
    for (i, pixel) in current.chunks_exact_mut(3).enumerate() {
        let x = i % grid.width;
        let y = i / grid.width;
        if y < 96 {
            pixel.copy_from_slice(&[(x * 13 + y * 17) as u8, (x * y) as u8, 211]);
        } else if x < 128 {
            pixel.copy_from_slice(&[29, 31, 37]);
        }
    }
    for phase in 0..2 {
        for budget in [32 * 1024, 128 * 1024, MAX_DELTA_BYTES] {
            let mut delta = Vec::new();
            if encode_delta(config, &previous, &current, &mut delta, budget).unwrap()
                == Delta::KeyframeRequired
            {
                assert!(delta.is_empty());
                let mut complete = Vec::new();
                assert!(!encode_keyframe(config, &current, &mut complete, budget).unwrap());
                assert!(complete.is_empty());
            }
        }
        if phase == 0 {
            previous.clone_from(&current);
            for pixel in current.chunks_exact_mut(3) {
                pixel[0] ^= 64;
            }
        }
    }
}
