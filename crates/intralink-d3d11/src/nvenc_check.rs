//! Native encoder diagnostic with a generated RGBA source, retained NV12 GPU
//! surface and synchronous H.264 output. No desktop capture or remote access.
use std::{
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::*};

use crate::{
    adapter::create_video_device,
    nvenc::NativeEncoder,
    scale::{PixelFormat, VideoProcessor},
};

const MAX_CLIP_BYTES: usize = 64 * 1024 * 1024;

/// Scan Annex B start codes without allocating a NAL list or interpreting
/// untrusted syntax. Used on native encoder output only.
fn nal_mask(bytes: &[u8]) -> u32 {
    let mut mask = 0u32;
    for window in bytes.windows(4) {
        if window[..3] == [0, 0, 1] && window[3] & 0x80 == 0 {
            mask |= 1 << (window[3] & 0x1f);
        }
    }
    mask
}

const IDR_HEADERS: u32 = (1 << 5) | (1 << 7) | (1 << 8);
const COLORS: [[u8; 4]; 6] = [
    [0, 0, 0, 255],
    [255, 255, 255, 255],
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [128, 128, 128, 255],
];

pub const FIXTURE_FPS: u32 = 60;
pub const FIXTURE_BITRATE: u32 = 8_000_000;

pub fn verify_nvenc(
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    output: Option<&Path>,
    decode: bool,
) -> Result<()> {
    run_fixture(
        height,
        samples,
        adapter_index,
        output,
        decode,
        |_, _| Ok(()),
    )
    .map(|_| ())
}

fn run_fixture(
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    output: Option<&Path>,
    decode: bool,
    mut consume: impl FnMut(bool, &[u8]) -> Result<()>,
) -> Result<ID3D11Device> {
    ensure!(
        matches!(height, 540 | 720 | 1080),
        "Use height 540, 720 or 1080"
    );
    ensure!((1..=180).contains(&samples), "Use 1-180 samples");
    let size = (u32::from(height) * 16 / 9, u32::from(height));
    let (w, h) = (size.0 as usize, size.1 as usize);
    let (device, context) = create_video_device("nvidia", adapter_index)?;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.0,
        Height: size.1,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        ..Default::default()
    };
    let mut texture = None;
    // SAFETY: descriptor is initialized and output storage lives through the call.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    let input = texture.context("Missing generated RGBA texture")?;
    let mut processor = VideoProcessor::new(
        &device,
        &context,
        &input,
        size,
        PixelFormat::Rgba,
        PixelFormat::Nv12,
    )?;
    let mut encoder =
        NativeEncoder::new(&device, processor.output(), FIXTURE_FPS, FIXTURE_BITRATE)?;
    let mut decoder = if decode {
        Some(super::nvdec::Decoder::new(&device, size)?)
    } else {
        None
    };
    let mut rgba = vec![0u8; w * h * 4];
    let mut packet = Vec::new();
    let mut clip = Vec::new();
    let mut lengths = 0usize;
    let mut keys = 0u32;
    let mut hardware_statuses = std::collections::BTreeMap::<u32, usize>::new();
    let mut times = Vec::with_capacity(usize::from(samples));
    for phase in 0..usize::from(samples) {
        // Generate changing full-height color bands; only upload synthetic pixels.
        let row = &mut rgba[..w * 4];
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&COLORS[(x * COLORS.len() / w + phase) % COLORS.len()]);
        }
        for y in 1..h {
            rgba.copy_within(0..w * 4, y * w * 4);
        }
        // SAFETY: initialized packed RGBA spans the whole subresource; all
        // immediate-context operations happen on this one calling thread.
        unsafe { context.UpdateSubresource(&input, 0, None, rgba.as_ptr().cast(), size.0 * 4, 0) };
        let start = Instant::now();
        processor.process()?;
        let force = phase == 0 || phase == usize::from(samples) / 2;
        let picture = encoder.encode(processor.output(), &mut packet, force)?;
        *hardware_statuses
            .entry(picture.hardware_status)
            .or_default() += 1;
        let key = picture.keyframe;
        times.push(start.elapsed());
        let mask = nal_mask(&packet);
        ensure!(
            mask & ((1 << 1) | (1 << 5)) != 0,
            "Missing coded picture in Annex B output"
        );
        if key {
            ensure!(mask & IDR_HEADERS == IDR_HEADERS, "IDR lacks SPS/PPS");
            keys += 1;
        }
        ensure!(!force || key, "Requested IDR missing");
        lengths = lengths
            .checked_add(packet.len())
            .context("Clip byte count overflow")?;
        ensure!(
            lengths <= MAX_CLIP_BYTES,
            "Generated clip exceeds diagnostic budget"
        );
        consume(key, &packet)?;
        if let Some(decoder) = &mut decoder {
            decoder.submit(&packet)?;
        }
        if output.is_some() {
            clip.extend_from_slice(&packet);
        }
    }
    encoder.finish()?;
    if let Some(decoder) = decoder {
        decoder.finish()?;
    }
    if let Some(path) = output {
        // Never overwrite an existing file. Only generated clip data is saved.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(&clip)?;
        file.sync_all()?;
    }
    times.sort_unstable();
    let percentile = |p: usize| -> f64 {
        let d: Duration = times[(times.len() * p).div_ceil(100) - 1];
        d.as_secs_f64() * 1000.0
    };
    println!(
        "nvenc_check=passed, size={}x{}, frames={samples}, idr={keys}, payload_bytes={lengths}, gpu_nv12_to_h264=true, cpu_pixel_readbacks=0, convert_encode_p50_ms={:.3}, p95_ms={:.3}, p99_ms={:.3}",
        w,
        h,
        percentile(50),
        percentile(95),
        percentile(99)
    );
    println!(
        "raw_hw_encode_status_counts={hardware_statuses:?}; opaque driver field, not API return codes"
    );
    println!(
        "Synthetic native H.264 diagnostic; native_decoder={decode}; no network or desktop capture."
    );
    Ok(device)
}

/// Owns only native-generated synthetic access units and the exact D3D11
/// device used to generate them. The diagnostic decoder never sees unmatched
/// network data, even when the transport itself is structurally valid.
pub struct EncodedFixture {
    device: ID3D11Device,
    size: (u32, u32),
    packets: Vec<(bool, Vec<u8>)>,
}

impl EncodedFixture {
    pub fn generate(height: u16, samples: u16, adapter_index: Option<u32>) -> Result<Self> {
        let mut packets = Vec::with_capacity(usize::from(samples.min(180)));
        let device = run_fixture(height, samples, adapter_index, None, false, |key, data| {
            packets.push((key, data.to_vec()));
            Ok(())
        })?;
        Ok(Self {
            device,
            size: (u32::from(height) * 16 / 9, u32::from(height)),
            packets,
        })
    }

    pub fn len(&self) -> usize {
        self.packets.len()
    }
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    pub fn packet(&self, index: usize) -> Option<(bool, &[u8])> {
        self.packets
            .get(index)
            .map(|(key, bytes)| (*key, bytes.as_slice()))
    }

    /// Decode only the exact previously generated fixture, on the original
    /// adapter. Compare each access unit again immediately before submission.
    pub fn verify_received<'a>(
        &self,
        received: impl ExactSizeIterator<Item = &'a [u8]>,
    ) -> Result<()> {
        ensure!(
            received.len() == self.packets.len() && !self.is_empty(),
            "Fixture receive count mismatch"
        );
        let mut decoder = super::nvdec::Decoder::new(&self.device, self.size)?;
        for (actual, (_, expected)) in received.zip(&self.packets) {
            ensure!(
                actual == expected,
                "Received bytes differ from generated fixture"
            );
            decoder.submit(actual)?;
        }
        decoder.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn annex_b_checks_required_nals_and_ignores_forbidden_bit() {
        assert_eq!(
            nal_mask(&[0, 0, 0, 1, 0x67, 3, 0, 0, 1, 0x68, 9, 0, 0, 1, 0x65]),
            IDR_HEADERS
        );
        assert_eq!(nal_mask(&[0, 0, 1, 0xe5]), 0);
        assert_eq!(nal_mask(&[0, 0, 1]), 0);
    }
}
