//! Reuse a staging texture across frames from one WGC capture device.
//! The mapped resource is never exposed to the caller.
use std::slice;

use anyhow::{Context, Result, ensure};
use windows::Win32::Graphics::{
    Direct3D11::{
        D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, ID3D11Device,
        ID3D11DeviceContext, ID3D11Texture2D,
    },
    Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_NV12, DXGI_FORMAT_R8G8B8A8_UNORM},
};
use windows_capture::{d3d11::StagingTexture, frame::Frame, settings::ColorFormat};

const MAX_PIXELS: u64 = 33_177_600;

#[derive(Default)]
pub struct Readback {
    staging: Option<StagingTexture>,
    device: Option<ID3D11Device>,
    allocations: u64,
}

impl Readback {
    /// Explicit local output retargeting only; retain device identity and counters.
    /// Ordinary readbacks still reject unexpected resource geometry changes.
    pub fn reset_output_staging(&mut self) {
        self.staging = None;
    }

    pub fn allocations(&self) -> u64 {
        self.allocations
    }

    pub fn copy_rgba(&mut self, frame: &Frame<'_>, output: &mut Vec<u8>) -> Result<()> {
        ensure!(
            frame.color_format() == ColorFormat::Rgba8,
            "Expected RGBA8 capture"
        );
        self.copy_texture(
            frame.device(),
            frame.device_context(),
            frame.as_raw_texture(),
            output,
        )
    }

    pub(crate) fn copy_texture(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        texture: &ID3D11Texture2D,
        output: &mut Vec<u8>,
    ) -> Result<()> {
        self.copy_formatted(device, context, texture, DXGI_FORMAT_R8G8B8A8_UNORM, output)
    }

    pub(crate) fn copy_nv12(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        texture: &ID3D11Texture2D,
        output: &mut Vec<u8>,
    ) -> Result<()> {
        self.copy_formatted(device, context, texture, DXGI_FORMAT_NV12, output)
    }

    fn copy_formatted(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        texture: &ID3D11Texture2D,
        format: DXGI_FORMAT,
        output: &mut Vec<u8>,
    ) -> Result<()> {
        // SAFETY: the live context and device are queried without mutating resources.
        unsafe {
            ensure!(
                &context.GetDevice()? == device,
                "D3D11 context belongs to a different device"
            );
            device.GetDeviceRemovedReason()?;
        }
        let desc = texture_desc_for(texture, device, format)?;
        let (width, height) = (desc.Width, desc.Height);
        let (row, rows, length) = format_layout(width, height, format)?;
        if let Some(previous) = &self.device {
            ensure!(previous == device, "D3D11 capture device changed");
        }
        if let Some(staging) = &self.staging {
            let previous = staging.desc();
            ensure!(
                previous.Width == width
                    && previous.Height == height
                    && previous.Format == desc.Format,
                "D3D11 source geometry or format changed"
            );
        } else {
            self.staging = Some(StagingTexture::new(device, width, height, desc.Format)?);
            self.device = Some(device.clone());
            self.allocations += 1;
        }
        let staging = self.staging.as_ref().context("Missing staging texture")?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: both resources are alive, owned by the same D3D11 device,
        // and have identical dimensions, format, array/mip and sample layout.
        // This callback serializes access; the previous map has been unmapped.
        unsafe {
            context.CopyResource(staging.texture(), texture);
            context.Map(staging.texture(), 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        }
        let _unmap = Unmap { context, staging };
        let pitch = mapped.RowPitch as usize;
        ensure!(!mapped.pData.is_null(), "Null D3D11 mapped pointer");
        let span = checked_span(row, pitch, rows)?;
        output.resize(length, 0);
        let base = mapped.pData.cast::<u8>();
        // D3D11 guarantees the mapped rows are valid until Unmap. Only read
        // initialized texels, not undefined row padding. The guard owns the map.
        for (y, destination) in output.chunks_exact_mut(row).enumerate() {
            let offset = y * pitch;
            debug_assert!(offset + row <= span);
            // SAFETY: checked_span proved offset arithmetic and isize limits;
            // the validated texture has height rows of pitch bytes.
            let source = unsafe { slice::from_raw_parts(base.add(offset), row) };
            destination.copy_from_slice(source);
        }
        drop(_unmap);
        // SAFETY: the live device is queried after the copy and unmap; a
        // removed device must not publish the just-read pixels.
        unsafe { device.GetDeviceRemovedReason()? };
        Ok(())
    }
}

pub(crate) fn texture_desc(
    texture: &ID3D11Texture2D,
    device: &ID3D11Device,
) -> Result<D3D11_TEXTURE2D_DESC> {
    texture_desc_for(texture, device, DXGI_FORMAT_R8G8B8A8_UNORM)
}

pub(crate) fn texture_desc_for(
    texture: &ID3D11Texture2D,
    device: &ID3D11Device,
    format: DXGI_FORMAT,
) -> Result<D3D11_TEXTURE2D_DESC> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: a live COM texture supplies its descriptor into a valid local
    // structure and GetDevice returns an owned reference to its device.
    unsafe {
        texture.GetDesc(&mut desc);
        ensure!(
            &texture.GetDevice()? == device,
            "D3D11 texture belongs to a different device"
        );
    }
    ensure!(
        desc.Format == format
            && desc.ArraySize == 1
            && desc.MipLevels == 1
            && desc.SampleDesc.Count == 1
            && desc.SampleDesc.Quality == 0,
        "Unsupported D3D11 texture format or layout"
    );
    format_layout(desc.Width, desc.Height, format)?;
    Ok(desc)
}

struct Unmap<'a> {
    context: &'a ID3D11DeviceContext,
    staging: &'a StagingTexture,
}

impl Drop for Unmap<'_> {
    fn drop(&mut self) {
        // SAFETY: this guard is created only after a successful Map, holds
        // both COM resources alive, and unmaps that exact subresource once.
        unsafe { self.context.Unmap(self.staging.texture(), 0) };
    }
}

pub(crate) fn layout(width: u32, height: u32) -> Result<(usize, usize)> {
    ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "Invalid D3D11 dimensions"
    );
    let row = (width as usize).checked_mul(4).context("Row overflow")?;
    let length = row.checked_mul(height as usize).context("Frame overflow")?;
    ensure!(
        length <= isize::MAX as usize,
        "Frame exceeds addressable range"
    );
    Ok((row, length))
}

/// Packed CPU plane layout. NV12 has height luma rows followed by height/2
/// interleaved UV rows, all with the same native pitch before copying.
pub(crate) fn format_layout(
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
) -> Result<(usize, usize, usize)> {
    let (rgba_row, rgba_len) = layout(width, height)?;
    if format == DXGI_FORMAT_R8G8B8A8_UNORM {
        return Ok((rgba_row, height as usize, rgba_len));
    }
    ensure!(format == DXGI_FORMAT_NV12, "Unsupported readback format");
    ensure!(
        width.is_multiple_of(2) && height.is_multiple_of(2),
        "NV12 dimensions must be even"
    );
    let rows = (height as usize)
        .checked_add(height as usize / 2)
        .context("NV12 rows overflow")?;
    let row = width as usize;
    let length = row.checked_mul(rows).context("NV12 length overflow")?;
    ensure!(
        length <= isize::MAX as usize,
        "NV12 exceeds addressable range"
    );
    Ok((row, rows, length))
}

fn checked_span(row: usize, pitch: usize, rows: usize) -> Result<usize> {
    ensure!(
        row > 0 && rows > 0 && pitch >= row,
        "Invalid D3D11 row pitch"
    );
    let span = pitch
        .checked_mul(rows - 1)
        .and_then(|n| n.checked_add(row))
        .context("Mapped span overflow")?;
    ensure!(
        span <= isize::MAX as usize,
        "Mapped span exceeds addressable range"
    );
    Ok(span)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12_layout_rejects_odd_and_invalid_dimensions_and_pitch() {
        assert_eq!(
            format_layout(1920, 1080, DXGI_FORMAT_NV12).unwrap(),
            (1920, 1620, 3_110_400)
        );
        assert_eq!(format_layout(4, 2, DXGI_FORMAT_NV12).unwrap(), (4, 3, 12));
        assert!(format_layout(3, 2, DXGI_FORMAT_NV12).is_err());
        assert!(format_layout(4, 3, DXGI_FORMAT_NV12).is_err());
        assert!(format_layout(0, 2, DXGI_FORMAT_NV12).is_err());
        assert!(format_layout(u32::MAX, 2, DXGI_FORMAT_NV12).is_err());
        assert_eq!(checked_span(4, 8, 3).unwrap(), 20);
        assert!(checked_span(4, 3, 3).is_err());
        assert!(checked_span(4, usize::MAX, 3).is_err());
    }

    #[test]
    fn readback_geometry_and_pitch_are_bounded() {
        assert_eq!(layout(1920, 1080).unwrap(), (7680, 8294400));
        assert!(layout(0, 1080).is_err());
        assert!(layout(u32::MAX, u32::MAX).is_err());
        assert_eq!(checked_span(12, 16, 2).unwrap(), 28);
        assert!(checked_span(12, 11, 2).is_err());
        assert!(checked_span(12, 16, 0).is_err());
        assert!(checked_span(12, usize::MAX, 2).is_err());
    }
}
