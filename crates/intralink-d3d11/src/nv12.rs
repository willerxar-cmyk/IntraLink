//! GPU-resident NV12 conversion and a synthetic native round-trip diagnostic.
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use windows::{
    Win32::Graphics::{
        Direct3D11::*,
        Dxgi::{Common::*, IDXGIDevice},
    },
    core::Interface,
};

use crate::{
    LatestSurface, Readback,
    scale::{PixelFormat, VideoProcessor},
};

/// Retains the input and one GPU output surface. No CPU readback on conversion.
pub struct GpuNv12 {
    processor: VideoProcessor,
}

/// Borrowed, session-owned GPU frame. The encoder must finish consuming it
/// before the next conversion. All immediate-context use stays serialized.
pub struct Nv12Surface<'a> {
    texture: &'a ID3D11Texture2D,
    size: (u32, u32),
}

impl Nv12Surface<'_> {
    pub fn texture(&self) -> &ID3D11Texture2D {
        self.texture
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Explicit limited-range BT.709, left-sited chroma.
    pub fn color_space(&self) -> DXGI_COLOR_SPACE_TYPE {
        DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709
    }
}

impl GpuNv12 {
    pub fn new(surface: &LatestSurface, size: (u32, u32)) -> Result<Self> {
        Ok(Self {
            processor: VideoProcessor::new(
                surface.device(),
                surface.context(),
                surface.texture(),
                size,
                PixelFormat::Rgba,
                PixelFormat::Nv12,
            )?,
        })
    }

    pub fn convert(&mut self, surface: &LatestSurface) -> Result<Nv12Surface<'_>> {
        self.processor.validate_source(surface)?;
        self.processor.process()?;
        Ok(Nv12Surface {
            texture: self.processor.output(),
            size: self.processor.output_size(),
        })
    }
}

const COLORS: [([u8; 4], [u8; 3]); 6] = [
    ([0, 0, 0, 255], [16, 128, 128]),
    ([255, 255, 255, 255], [235, 128, 128]),
    ([255, 0, 0, 255], [63, 102, 240]),
    ([0, 255, 0, 255], [173, 42, 26]),
    ([0, 0, 255, 255], [32, 240, 118]),
    ([128, 128, 128, 255], [126, 128, 128]),
];

/// Hardware-only diagnostic. Upload generated color bands and reuse the same
/// input, two processor outputs and two staging allocations across all samples.
pub fn verify_nv12(
    height: u16,
    samples: u16,
    vendor: &str,
    adapter_index: Option<u32>,
) -> Result<()> {
    ensure!(matches!(height, 540 | 1080), "Use height 540 or 1080");
    ensure!((1..=180).contains(&samples), "Use 1-180 samples");
    let size = (u32::from(height) * 16 / 9, u32::from(height));
    let (w, h) = (size.0 as usize, size.1 as usize);
    let (device, context) = crate::adapter::create_video_device(vendor, adapter_index)?;
    let dxgi: IDXGIDevice = device.cast()?;
    // SAFETY: live COM interfaces are queried, not modified.
    let adapter = unsafe { dxgi.GetAdapter()?.GetDesc()? };
    let name = String::from_utf16_lossy(&adapter.Description)
        .trim_end_matches('\0')
        .to_owned();
    println!(
        "adapter={name}, vendor_id={:#06x}, device_id={:#06x}",
        adapter.VendorId, adapter.DeviceId
    );
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
    // SAFETY: the descriptor is initialized and the output is a live local.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    let input = texture.context("Missing synthetic input texture")?;
    let mut forward = VideoProcessor::new(
        &device,
        &context,
        &input,
        size,
        PixelFormat::Rgba,
        PixelFormat::Nv12,
    )?;
    let mut reverse = VideoProcessor::new(
        &device,
        &context,
        forward.output(),
        size,
        PixelFormat::Nv12,
        PixelFormat::Rgba,
    )?;
    let mut rgba = vec![0; w * h * 4];
    let mut nv12 = Vec::new();
    let mut restored = Vec::new();
    let mut nv_readback = Readback::default();
    let mut rgb_readback = Readback::default();
    let mut max_yuv_error = 0u8;
    let mut max_rgb_error = 0u8;
    let mut forward_times = Vec::with_capacity(usize::from(samples));
    let mut reverse_times = Vec::with_capacity(usize::from(samples));
    for phase in 0..usize::from(samples) {
        // Shift the colors on each sample so stale outputs cannot pass.
        for row in rgba.chunks_exact_mut(w * 4) {
            for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
                pixel.copy_from_slice(&COLORS[(x * COLORS.len() / w + phase) % COLORS.len()].0);
            }
        }
        // SAFETY: initialized packed RGBA bytes cover the complete input
        // subresource. This diagnostic owns the device/context on one thread.
        unsafe { context.UpdateSubresource(&input, 0, None, rgba.as_ptr().cast(), size.0 * 4, 0) };
        let start = Instant::now();
        forward.process()?;
        nv_readback.copy_nv12(&device, &context, forward.output(), &mut nv12)?;
        forward_times.push(start.elapsed());
        ensure!(nv12.len() == w * h * 3 / 2, "Unexpected NV12 plane lengths");
        let start = Instant::now();
        reverse.process()?;
        rgb_readback.copy_texture(&device, &context, reverse.output(), &mut restored)?;
        reverse_times.push(start.elapsed());
        ensure!(
            restored.len() == rgba.len(),
            "Unexpected RGB round-trip length"
        );
        for band in 0..COLORS.len() {
            let x = ((2 * band + 1) * w / (2 * COLORS.len())) & !1;
            let (rgb, yuv) = COLORS[(band + phase) % COLORS.len()];
            // Validate multiple interior rows, away from chroma boundaries.
            for y in [h / 4, h / 2, h * 3 / 4] {
                let uv = w * h + (y / 2) * w + x;
                let observed = [nv12[y * w + x], nv12[uv], nv12[uv + 1]];
                for (actual, expected) in observed.into_iter().zip(yuv) {
                    max_yuv_error = max_yuv_error.max(actual.abs_diff(expected));
                    ensure!(
                        actual.abs_diff(expected) <= 3,
                        "BT.709 NV12 mismatch: sample={phase}, band={band}, observed={observed:?}, expected={yuv:?}"
                    );
                }
                let offset = (y * w + x) * 4;
                for (actual, expected) in restored[offset..offset + 3].iter().zip(rgb) {
                    max_rgb_error = max_rgb_error.max(actual.abs_diff(expected));
                    ensure!(
                        actual.abs_diff(expected) <= 6,
                        "NV12 round-trip mismatch: sample={phase}, band={band}, observed={:?}, expected={rgb:?}",
                        &restored[offset..offset + 3]
                    );
                }
            }
        }
    }
    ensure!(
        nv_readback.allocations() == 1 && rgb_readback.allocations() == 1,
        "Staging was not reused"
    );
    forward_times.sort_unstable();
    reverse_times.sort_unstable();
    let median = |times: &[std::time::Duration]| times[times.len() / 2].as_secs_f64() * 1000.0;
    println!(
        "nv12_check=passed, size={}x{}, samples={samples}, color=BT709_limited_left, packed_bytes={}, max_yuv_error={max_yuv_error}, max_rgb_error={max_rgb_error}, staging_allocations=2, forward_with_readback_p50_ms={:.3}, reverse_with_readback_p50_ms={:.3}",
        w,
        h,
        nv12.len(),
        median(&forward_times),
        median(&reverse_times)
    );
    println!("Synthetic GPU color conversion only; no video codec, desktop capture or network.");
    Ok(())
}
