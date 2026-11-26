//! Deterministic CPU keyframe benchmark; no desktop capture, network or GPU.
use std::{hint::black_box, time::Instant};

use intralink_core::{
    desktop::{CodecMode, VideoConfig},
    media::{apply_keyframe, encode_keyframe},
};

const ATTEMPT_BUDGET: usize = 128 * 1024;
const RUNS: u32 = 30;

fn main() {
    println!("resolution,scene,accepted,payload_bytes,encode_mean_us,decode_mean_us");
    for height in [540, 720, 1080] {
        let config = VideoConfig::new(height, 30, 75, 30)
            .expect("valid profile")
            .with_mode(CodecMode::Compact);
        let width = config.width() as usize;
        let height = config.height() as usize;
        let size = width * height;
        for scene in ["uniform", "ui-like", "high-entropy"] {
            let mut rgb = vec![0; size * 3];
            let mut state = 0x1234_5678u32;
            for (i, pixel) in rgb.chunks_exact_mut(3).enumerate() {
                let (x, y) = (i % width, i / width);
                let color = match scene {
                    "uniform" => [17, 43, 199],
                    "ui-like" if y < 64 => [16, 32, 48],
                    "ui-like" if x < width / 4 => [40, 58, 76],
                    "ui-like" if y % 32 < 2 && x % 80 < 70 => [20, 50, 80],
                    "ui-like" => [232, 236, 240],
                    _ => {
                        let mut color = [0; 3];
                        for byte in &mut color {
                            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                            *byte = (state >> 24) as u8;
                        }
                        color
                    }
                };
                pixel.copy_from_slice(&color);
            }
            let mut encoded = Vec::new();
            let accepted = encode_keyframe(config, &rgb, &mut encoded, ATTEMPT_BUDGET)
                .expect("warm-up encode");
            let start = Instant::now();
            for _ in 0..RUNS {
                let actual = encode_keyframe(config, black_box(&rgb), &mut encoded, ATTEMPT_BUDGET)
                    .expect("encode");
                assert_eq!(actual, accepted);
                let _ = black_box(&encoded);
            }
            let encode_us = start.elapsed().as_secs_f64() * 1e6 / f64::from(RUNS);
            let decode_us = if accepted {
                let mut pixels = Vec::new();
                apply_keyframe(config, &encoded, &mut pixels).expect("warm-up decode");
                let start = Instant::now();
                for _ in 0..RUNS {
                    apply_keyframe(config, black_box(&encoded), &mut pixels).expect("decode");
                    let _ = black_box(&pixels);
                }
                let elapsed = start.elapsed();
                assert_eq!(pixels.len(), size);
                assert!(
                    pixels
                        .iter()
                        .zip(rgb.chunks_exact(3))
                        .all(|(pixel, color)| {
                            *pixel
                                == ((u32::from(color[0]) << 16)
                                    | (u32::from(color[1]) << 8)
                                    | u32::from(color[2]))
                        })
                );
                format!("{:.2}", elapsed.as_secs_f64() * 1e6 / f64::from(RUNS))
            } else {
                assert!(encoded.is_empty());
                "n/a".to_owned()
            };
            println!(
                "{width}x{height},{scene},{accepted},{},{encode_us:.2},{decode_us}",
                encoded.len()
            );
        }
    }
}
