//! Standalone CPU microbenchmark of the actual presentation surface.
//! No native capture, codec, network, GPU, GUI or timed 60 FPS schedule.
#![forbid(unsafe_code)]

#[path = "../crates/intralink-core/src/presentation.rs"]
pub mod presentation;

use std::{hint::black_box, time::Instant};
use presentation::{PresentationSurface, SurfaceError};

const DIMENSIONS: (usize, usize) = (1920, 1080);
const PIXELS: usize = DIMENSIONS.0 * DIMENSIONS.1;
const DEVICES: usize = 3;
const ROUNDS: usize = 240;

fn run(overlap: bool) -> Result<(), SurfaceError> {
    let mut surfaces: [PresentationSurface; DEVICES] = std::array::from_fn(|_| PresentationSurface::default());
    for (device, surface) in surfaces.iter_mut().enumerate() {
        surface.update(1, DIMENSIONS, |pixels| {
            pixels.resize(PIXELS, device as u32);
            Ok::<(), SurfaceError>(())
        })?;
    }
    let mut copies = 0usize;
    let start = Instant::now();
    for round in 0..ROUNDS {
        let focused = round % DEVICES;
        let held = overlap.then(|| surfaces[focused].snapshot_after(0).expect("Initialized surface"));
        for (device, surface) in surfaces.iter_mut().enumerate() {
            let index = (round * 7919 + device) % PIXELS;
            surface.update(round + 2, DIMENSIONS, |pixels| {
                pixels[index] = (round + 100) as u32;
                black_box(pixels[index]);
                Ok::<(), SurfaceError>(())
            })?;
        }
        let current = surfaces[focused].snapshot_after(0).expect("Updated surface");
        if let Some(old) = held {
            copies += usize::from(old.pixels().as_ptr() != current.pixels().as_ptr());
            black_box(old.pixels()[round % PIXELS]);
        }
        black_box(current.pixels()[(round * 7919 + focused) % PIXELS]);
    }
    let elapsed = start.elapsed();
    println!("overlap={overlap}, devices={DEVICES}, rounds={ROUNDS}, updates={}, copy_on_write_events={copies}, elapsed_ms={:.3}, mean_us_per_round={:.3}", DEVICES * ROUNDS, elapsed.as_secs_f64() * 1000.0, elapsed.as_secs_f64() * 1_000_000.0 / ROUNDS as f64);
    Ok(())
}

fn main() -> Result<(), SurfaceError> {
    run(false)?;
    run(true)
}
