//! One owned latest-frame GPU surface. No CPU readback is performed on push.
use anyhow::{Context, Result, ensure};
use windows::Win32::Graphics::{
    Direct3D11::{
        D3D11_BIND_RENDER_TARGET, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device,
        ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
    },
    Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC},
};
use windows::core::Interface;
use windows_capture::{frame::Frame, settings::ColorFormat};

use crate::readback::{Readback, texture_desc};

/// Owns a copy of the most recent complete frame, independent of WGC's
/// frame-pool lifetime. All immediate-context access must be serialized by
/// the same capture handler lock, including consuming this surface.
pub struct LatestSurface {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    texture: ID3D11Texture2D,
    size: (u32, u32),
}

impl LatestSurface {
    pub fn new(frame: &Frame<'_>) -> Result<Self> {
        ensure!(
            frame.color_format() == ColorFormat::Rgba8,
            "Expected RGBA8 capture"
        );
        Self::from_texture(
            frame.device(),
            frame.device_context(),
            frame.as_raw_texture(),
        )
    }

    pub(crate) fn from_texture(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        texture: &ID3D11Texture2D,
    ) -> Result<Self> {
        protect_context(context)?;
        // SAFETY: query the owning device of the live immediate context.
        let owner = unsafe { context.GetDevice()? };
        ensure!(&owner == device, "Capture context device mismatch");
        let source = texture_desc(texture, device)?;
        let size = (source.Width, source.Height);
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
        let mut retained = None;
        // SAFETY: the validated source determines the initialized descriptor;
        // COM output storage and the capture device remain alive for the call.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut retained))? };
        let mut surface = Self {
            device: device.clone(),
            context: context.clone(),
            texture: retained.context("Missing latest-frame texture")?,
            size,
        };
        surface.update_texture(device, context, texture)?;
        Ok(surface)
    }

    pub fn update(&mut self, frame: &Frame<'_>) -> Result<()> {
        ensure!(
            frame.color_format() == ColorFormat::Rgba8,
            "Capture format changed"
        );
        self.update_texture(
            frame.device(),
            frame.device_context(),
            frame.as_raw_texture(),
        )
    }

    pub(crate) fn update_texture(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        texture: &ID3D11Texture2D,
    ) -> Result<()> {
        self.check_device()?;
        let desc = texture_desc(texture, &self.device)?;
        ensure!(
            self.size == (desc.Width, desc.Height)
                && &self.device == device
                && &self.context == context,
            "WGC source geometry or device changed"
        );
        // SAFETY: both textures belong to the same device, have validated
        // identical dimensions and subresource layout, and remain live.
        // The caller serializes this context and the retained surface.
        unsafe { self.context.CopyResource(&self.texture, texture) };
        self.check_device()
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn check_device(&self) -> Result<()> {
        // SAFETY: the retained COM device is live; this query does not mutate resources.
        unsafe { self.device.GetDeviceRemovedReason()? };
        Ok(())
    }

    pub fn read(&self, readback: &mut Readback, rgba: &mut Vec<u8>) -> Result<()> {
        self.check_device()?;
        readback.copy_texture(&self.device, &self.context, &self.texture, rgba)
    }

    pub(crate) fn device(&self) -> &ID3D11Device {
        &self.device
    }
    pub(crate) fn context(&self) -> &ID3D11DeviceContext {
        &self.context
    }
    pub(crate) fn texture(&self) -> &ID3D11Texture2D {
        &self.texture
    }
}

/// Enable native immediate-context serialization in addition to the handler
/// lock. The context is used by the WGC callback and the requesting thread.
fn protect_context(context: &ID3D11DeviceContext) -> Result<()> {
    let protection: ID3D11Multithread = context
        .cast()
        .context("D3D11 context protection unavailable")?;
    // SAFETY: this is the live, session-owned context; enabling the native
    // lock is supported by ID3D11Multithread and does not replace the outer
    // handler lock that keeps multi-operation transactions coherent.
    unsafe {
        // The returned BOOL is the previous protection state, not success.
        let _previous_protection = protection.SetMultithreadProtected(true);
        ensure!(
            protection.GetMultithreadProtected().as_bool(),
            "D3D11 context protection failed"
        );
    }
    Ok(())
}
