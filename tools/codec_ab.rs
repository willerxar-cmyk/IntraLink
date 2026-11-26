//! Interleaved, same-process A/B comparison for the 0.18 and 0.19 codecs.
//! See docs/CORE-VALIDATION.md for preparing the exact module snapshots.
#![forbid(unsafe_code)]

use std::{hint::black_box, time::Instant};
pub mod desktop;
pub mod old;
pub mod new;

use desktop::{CodecMode, VideoConfig};
const ROUNDS: usize = 41;
const BUDGET: usize = 128 * 1024;

fn measure(samples: &mut [u128]) -> (f64, f64) {
    samples.sort_unstable();
    (samples[samples.len() / 2] as f64 / 1000.0,
     samples[(samples.len() * 95).div_ceil(100) - 1] as f64 / 1000.0)
}

fn main() {
    let config = VideoConfig::new(1080, 60, 75, 30).unwrap().with_mode(CodecMode::Compact);
    let (width, height) = (config.width() as usize, config.height() as usize);
    println!("scene,accepted,payload_bytes,old_median_us,new_median_us,old_p95_us,new_p95_us");
    for scene in ["uniform", "ui-like", "high-entropy"] {
        let mut rgb = vec![0; width * height * 3];
        let mut state = 0x1234_5678u32;
        for (i, pixel) in rgb.chunks_exact_mut(3).enumerate() {
            let (x, y) = (i % width, i / width);
            let color = match scene {
                "uniform" => [17, 43, 199],
                "ui-like" if y < 64 => [16, 32, 48],
                "ui-like" if x < width / 4 => [40, 58, 76],
                "ui-like" if y % 32 < 2 && x % 80 < 70 => [20, 50, 80],
                "ui-like" => [232, 236, 240],
                _ => std::array::from_fn(|_| {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    (state >> 24) as u8
                }),
            };
            pixel.copy_from_slice(&color);
        }
        let mut a = Vec::new();
        let mut b = Vec::new();
        let accepted = old::encode_keyframe(config, &rgb, &mut a, BUDGET).unwrap();
        assert_eq!(new::encode_keyframe(config, &rgb, &mut b, BUDGET).unwrap(), accepted);
        assert_eq!(a, b);
        let mut old_times = [0; ROUNDS];
        let mut new_times = [0; ROUNDS];
        for round in 0..ROUNDS {
            let mut run_old = || {
                let start = Instant::now();
                let result = old::encode_keyframe(config, black_box(&rgb), &mut a, BUDGET).unwrap();
                old_times[round] = start.elapsed().as_nanos();
                assert_eq!(result, accepted);
            };
            let mut run_new = || {
                let start = Instant::now();
                let result = new::encode_keyframe(config, black_box(&rgb), &mut b, BUDGET).unwrap();
                new_times[round] = start.elapsed().as_nanos();
                assert_eq!(result, accepted);
            };
            if round % 2 == 0 { run_old(); run_new(); }
            else { run_new(); run_old(); }
            assert_eq!(a, b);
            black_box(&a);
            black_box(&b);
        }
        if accepted {
            let mut pixels = Vec::new();
            new::apply_keyframe(config, &b, &mut pixels).unwrap();
            assert_eq!(pixels.len(), width * height);
            assert!(pixels.iter().zip(rgb.chunks_exact(3)).all(|(p, c)| {
                *p == (u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]))
            }));
        }
        let (a_med, a95) = measure(&mut old_times);
        let (b_med, b95) = measure(&mut new_times);
        println!("{scene},{accepted},{},{a_med:.2},{b_med:.2},{a95:.2},{b95:.2}", b.len());
    }
}
