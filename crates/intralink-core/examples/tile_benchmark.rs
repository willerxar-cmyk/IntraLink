//! Local deterministic microbenchmark. No screen, network, or external process.
use std::{hint::black_box, time::Instant};

use intralink_core::{
    desktop::{CodecMode, VideoConfig},
    media::{Delta, MAX_DELTA_BYTES, apply_delta, encode_delta},
};

fn main() {
    println!("resolution,scene,mode,result,payload_bytes,encode_mean_us,decode_mean_us");
    for height in [540, 720, 1080] {
        let config = VideoConfig::new(height, 30, 75, 30).expect("valid profile");
        let width = config.width() as usize;
        let height = config.height() as usize;
        let size = width * height;
        let previous = vec![0u8; size * 3];
        for scene in [
            "static",
            "solid-tile",
            "text-like-tile",
            "stripe-tile",
            "photo-like-tile",
            "full-motion",
        ] {
            let mut current = previous.clone();
            match scene {
                "static" => {}
                "solid-tile" | "text-like-tile" | "stripe-tile" | "photo-like-tile" => {
                    for y in 0..32 {
                        for x in 0..32 {
                            let rgb = match scene {
                                "photo-like-tile" => {
                                    [(x * 7) as u8, (y * 7) as u8, ((x + y) * 3) as u8]
                                }
                                "solid-tile" => [210, 30, 10],
                                "stripe-tile" if y < 16 => [210, 30, 10],
                                "text-like-tile" if (x + y) % 3 == 0 => [210, 30, 10],
                                _ => [0, 0, 0],
                            };
                            let start = (y * width + x) * 3;
                            current[start..start + 3].copy_from_slice(&rgb);
                        }
                    }
                }
                "full-motion" => {
                    for (i, pixel) in current.chunks_exact_mut(3).enumerate() {
                        pixel.copy_from_slice(&[(i % 251) as u8, ((i / 251) % 251) as u8, 127]);
                    }
                }
                _ => unreachable!(),
            }
            for mode in [CodecMode::Tiles, CodecMode::Compact] {
                let config = config.with_mode(mode);
                let mut payload = Vec::new();
                let mut pixels = vec![0; size];
                let runs = 100;
                // Warm-up and initial allocation are excluded from timing.
                encode_delta(config, &previous, &current, &mut payload, MAX_DELTA_BYTES)
                    .expect("warm-up");
                let start = Instant::now();
                let mut result = Delta::Unchanged;
                for _ in 0..runs {
                    result = encode_delta(
                        config,
                        black_box(&previous),
                        black_box(&current),
                        &mut payload,
                        MAX_DELTA_BYTES,
                    )
                    .expect("encode");
                    let _ = black_box(&payload);
                }
                let encode_us = start.elapsed().as_secs_f64() * 1e6 / f64::from(runs);
                let decode_us = if result == Delta::Encoded {
                    apply_delta(config, &payload, &mut pixels).expect("warm-up");
                    let start = Instant::now();
                    for _ in 0..runs {
                        apply_delta(config, black_box(&payload), &mut pixels).expect("decode");
                        let _ = black_box(&pixels);
                    }
                    let elapsed = start.elapsed();
                    assert!(
                        pixels
                            .iter()
                            .zip(current.chunks_exact(3))
                            .all(|(pixel, rgb)| {
                                *pixel
                                    == ((u32::from(rgb[0]) << 16)
                                        | (u32::from(rgb[1]) << 8)
                                        | u32::from(rgb[2]))
                            })
                    );
                    format!("{:.2}", elapsed.as_secs_f64() * 1e6 / f64::from(runs))
                } else {
                    "n/a".to_owned()
                };
                println!(
                    "{width}x{height},{scene},{mode:?},{result:?},{},{encode_us:.2},{decode_us}",
                    payload.len()
                );
            }
        }
    }
}
