//! Reference pixels for the generated Android DenseMotionFixture only.
//! Measure RGB distortion in the dense phase; verify every sparse pixel.
use anyhow::{Result, ensure};

#[derive(Default)]
pub(super) struct Quality {
    squared: u64,
    channels: u64,
    dense_frames: usize,
    dense_bytes: u64,
    sparse_frames: usize,
    sparse_pixels: u64,
    sparse_mismatch: u64,
    sparse_bytes: u64,
}

fn pixel(frame: usize, x: usize, y: usize) -> u32 {
    let marker = frame * 17 % (1920 - 128);
    if (marker..marker + 128).contains(&x) && (200..500).contains(&y) {
        return 0x68d8bf;
    }
    if !(10..150).contains(&frame) {
        return 0x142436;
    }
    let x = (x + frame * 19) % 1920;
    let r = (x * 3 + y * 2 + (x / 24 % 2) * 70) & 255;
    let g = (x + y * 5 + (y / 32 % 2) * 64) & 255;
    let b = (x * 2 + y * 3 + (x / 64 % 2) * 92) & 255;
    ((r << 16) | (g << 8) | b) as u32
}

impl Quality {
    pub(super) fn observe(
        &mut self,
        frame: usize,
        dimensions: (usize, usize),
        pixels: &[u32],
        bytes: usize,
    ) -> Result<()> {
        ensure!(
            frame < 210 && dimensions == (1920, 1080) && pixels.len() == 1920 * 1080,
            "Wrong dense fixture geometry"
        );
        if (10..150).contains(&frame) {
            self.dense_frames += 1;
            self.dense_bytes += bytes as u64;
            for y in (3..1080).step_by(16) {
                for x in (5..1920).step_by(16) {
                    let a = pixels[y * 1920 + x];
                    let b = pixel(frame, x, y);
                    for shift in [0, 8, 16] {
                        let delta = ((a >> shift) & 255) as i32 - ((b >> shift) & 255) as i32;
                        self.squared += (delta * delta) as u64;
                        self.channels += 1;
                    }
                }
            }
        } else {
            self.sparse_frames += 1;
            self.sparse_bytes += bytes as u64;
            for (i, &actual) in pixels.iter().enumerate() {
                self.sparse_pixels += 1;
                self.sparse_mismatch += u64::from(actual != pixel(frame, i % 1920, i / 1920));
            }
        }
        Ok(())
    }

    pub(super) fn report(&self) -> Result<()> {
        ensure!(
            self.dense_frames == 140 && self.sparse_frames == 70 && self.channels > 0,
            "Incomplete quality measurement"
        );
        let mse = self.squared as f64 / self.channels as f64;
        let psnr = if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0 * 255.0 / mse).log10()
        };
        println!(
            "DENSE_QUALITY dense_frames={} dense_bytes={} sparse_frames={} sparse_bytes={} sparse_pixels={} sparse_mismatch={} samples={} psnr_db={psnr:.3}",
            self.dense_frames,
            self.dense_bytes,
            self.sparse_frames,
            self.sparse_bytes,
            self.sparse_pixels,
            self.sparse_mismatch,
            self.channels
        );
        ensure!(
            self.sparse_mismatch == 0,
            "Generated sparse UI did not recover exact pixels"
        );
        Ok(())
    }
}
