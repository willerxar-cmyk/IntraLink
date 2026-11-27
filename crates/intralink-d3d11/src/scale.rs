//! Shared D3D11 video processor, with explicit SDR color metadata.
use std::{
    mem::ManuallyDrop,
    time::{Duration, Instant},
};

use crate::{
    LatestSurface,
    readback::{Readback, format_layout, texture_desc_for},
};
use anyhow::{Context, Result, ensure};
use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::{
            Direct3D11::*,
            Dxgi::Common::{
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_TYPE,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709, DXGI_FORMAT, DXGI_FORMAT_NV12,
                DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
            },
        },
    },
    core::{BOOL, Interface},
};

#[derive(Clone, Copy)]
pub(crate) enum PixelFormat {
    Rgba,
    Nv12,
}

impl PixelFormat {
    fn dxgi(self) -> DXGI_FORMAT {
        match self {
            Self::Rgba => DXGI_FORMAT_R8G8B8A8_UNORM,
            Self::Nv12 => DXGI_FORMAT_NV12,
        }
    }

    fn color(self) -> DXGI_COLOR_SPACE_TYPE {
        match self {
            Self::Rgba => DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            Self::Nv12 => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
        }
    }
}

pub(crate) struct VideoProcessor {
    device: ID3D11Device,
    immediate: ID3D11DeviceContext,
    context: ID3D11VideoContext1,
    processor: ID3D11VideoProcessor,
    input: ID3D11Texture2D,
    input_view: ID3D11VideoProcessorInputView,
    output: ID3D11Texture2D,
    output_view: ID3D11VideoProcessorOutputView,
    source_size: (u32, u32),
    output_size: (u32, u32),
}

impl VideoProcessor {
    pub(crate) fn new(
        device: &ID3D11Device,
        immediate: &ID3D11DeviceContext,
        input: &ID3D11Texture2D,
        output_size: (u32, u32),
        input_format: PixelFormat,
        output_format: PixelFormat,
    ) -> Result<Self> {
        // SAFETY: all interfaces are live. Verify the owning device before use.
        unsafe {
            ensure!(
                &immediate.GetDevice()? == device,
                "Video context device mismatch"
            );
            device.GetDeviceRemovedReason()?;
        }
        let desc = texture_desc_for(input, device, input_format.dxgi())?;
        let source_size = (desc.Width, desc.Height);
        format_layout(output_size.0, output_size.1, output_format.dxgi())?;
        ensure!(
            output_size.0 <= source_size.0 && output_size.1 <= source_size.1,
            "GPU downscaling does not upscale the captured image"
        );
        let vd: ID3D11VideoDevice = device.cast().context("D3D11 video device unavailable")?;
        let context: ID3D11VideoContext1 = immediate
            .cast()
            .context("D3D11 video context 1 unavailable")?;
        let rate = DXGI_RATIONAL {
            Numerator: 60,
            Denominator: 1,
        };
        let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: source_size.0,
            InputHeight: source_size.1,
            OutputFrameRate: rate,
            OutputWidth: output_size.0,
            OutputHeight: output_size.1,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
        };
        let in_format = input_format.dxgi();
        let out_format = output_format.dxgi();
        let in_color = input_format.color();
        let out_color = output_format.color();
        // SAFETY: all descriptors have validated nonzero dimensions; COM
        // interfaces are live references to the capture device.
        let enumerator = unsafe { vd.CreateVideoProcessorEnumerator(&content)? };
        // SAFETY: enumerator was successfully constructed for this content.
        let (input_support, output_support) = unsafe {
            (
                enumerator.CheckVideoProcessorFormat(in_format)?,
                enumerator.CheckVideoProcessorFormat(out_format)?,
            )
        };
        ensure!(
            input_support & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0 as u32 != 0
                && output_support & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0 as u32 != 0,
            "D3D11 processor lacks requested input/output format"
        );
        let modern: ID3D11VideoProcessorEnumerator1 = enumerator
            .cast()
            .context("Explicit video color-space negotiation unavailable")?;
        // SAFETY: the live enumerator queries the validated format and color spaces.
        let conversion = unsafe {
            modern
                .CheckVideoProcessorFormatConversion(in_format, in_color, out_format, out_color)?
        };
        ensure!(
            conversion.as_bool(),
            "D3D11 processor lacks requested explicit SDR color conversion"
        );
        // SAFETY: the returned enumerator supports the validated formats.
        // Index zero is the first advertised processor configuration.
        let processor = unsafe { vd.CreateVideoProcessor(&enumerator, 0)? };
        let input = input.clone();
        let output = texture(device, output_size, out_format)?;
        let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: 0,
                },
            },
        };
        let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        let mut iv = None;
        let mut ov = None;
        let source_rect = rect(source_size);
        let output_rect = rect(output_size);
        // SAFETY: the textures belong to this device, the views refer to
        // subresource zero, and all rect pointers live through these calls.
        // No automatic enhancement, HDR conversion or temporal processing.
        unsafe {
            vd.CreateVideoProcessorInputView(&input, &enumerator, &input_desc, Some(&mut iv))?;
            vd.CreateVideoProcessorOutputView(&output, &enumerator, &output_desc, Some(&mut ov))?;
            context.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            context.VideoProcessorSetStreamColorSpace1(&processor, 0, in_color);
            context.VideoProcessorSetOutputColorSpace1(&processor, out_color);
            context.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&source_rect));
            context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&output_rect));
            context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&output_rect));
        }
        Ok(Self {
            device: device.clone(),
            immediate: immediate.clone(),
            context,
            processor,
            input,
            input_view: iv.context("Missing video input view")?,
            output,
            output_view: ov.context("Missing video output view")?,
            source_size,
            output_size,
        })
    }

    pub(crate) fn output_size(&self) -> (u32, u32) {
        self.output_size
    }

    pub(crate) fn output(&self) -> &ID3D11Texture2D {
        &self.output
    }

    pub(crate) fn validate_source(&self, surface: &LatestSurface) -> Result<()> {
        surface.check_device()?;
        ensure!(
            self.source_size == surface.size()
                && &self.device == surface.device()
                && &self.immediate == surface.context()
                && &self.input == surface.texture(),
            "GPU capture source changed"
        );
        Ok(())
    }

    /// Submit the bound input to the retained output. The caller serializes
    /// all operations on the immediate context; no CPU readback occurs here.
    pub(crate) fn process(&mut self) -> Result<()> {
        // SAFETY: the retained device is live; this query detects known removal.
        unsafe { self.device.GetDeviceRemovedReason()? };
        let stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: BOOL(1),
            pInputSurface: ManuallyDrop::new(Some(self.input_view.clone())),
            ..Default::default()
        };
        // SAFETY: the input view references the retained latest surface;
        // all views remain live and the handler lock serializes context use.
        let result = unsafe {
            self.context.VideoProcessorBlt(
                &self.processor,
                &self.output_view,
                0,
                std::slice::from_ref(&stream),
            )
        };
        // The generated ABI field is ManuallyDrop: release our cloned COM
        // reference even if VideoProcessorBlt failed.
        drop(ManuallyDrop::into_inner(stream.pInputSurface));
        result?;
        // SAFETY: the retained device is live, checked again after submission.
        unsafe { self.device.GetDeviceRemovedReason()? };
        Ok(())
    }
}

/// Existing RGBA scaler using the shared native video processor.
pub struct GpuScaler {
    processor: VideoProcessor,
}

impl GpuScaler {
    pub fn new(surface: &LatestSurface, output_size: (u32, u32)) -> Result<Self> {
        Ok(Self {
            processor: VideoProcessor::new(
                surface.device(),
                surface.context(),
                surface.texture(),
                output_size,
                PixelFormat::Rgba,
                PixelFormat::Rgba,
            )?,
        })
    }

    pub fn output_size(&self) -> (u32, u32) {
        self.processor.output_size()
    }

    /// Submit scale and copy under the source lock. The pending staging
    /// resource is independent of later WGC updates on the same context.
    pub fn submit_readback(
        &mut self,
        surface: &LatestSurface,
        readback: &mut Readback,
    ) -> Result<Duration> {
        ensure!(
            !readback.is_pending(),
            "Cannot scale while readback is pending"
        );
        let started = Instant::now();
        self.processor.validate_source(surface)?;
        self.processor.process()?;
        let scale_submit = started.elapsed();
        readback.begin_texture(surface.device(), surface.context(), self.processor.output())?;
        Ok(scale_submit)
    }

    pub fn scale_and_read(
        &mut self,
        surface: &LatestSurface,
        readback: &mut Readback,
        rgba: &mut Vec<u8>,
    ) -> Result<Duration> {
        ensure!(
            !readback.is_pending(),
            "Cannot scale while readback is pending"
        );
        let started = Instant::now();
        self.processor.validate_source(surface)?;
        self.processor.process()?;
        let scale_submit = started.elapsed();
        readback.copy_texture(
            surface.device(),
            surface.context(),
            self.processor.output(),
            rgba,
        )?;
        Ok(scale_submit)
    }
}

fn rect(size: (u32, u32)) -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: size.0 as i32,
        bottom: size.1 as i32,
    }
}

fn texture(
    device: &ID3D11Device,
    size: (u32, u32),
    format: DXGI_FORMAT,
) -> Result<ID3D11Texture2D> {
    format_layout(size.0, size.1, format)?;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.0,
        Height: size.1,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        ..Default::default()
    };
    let mut result = None;
    // SAFETY: descriptor is initialized and the output storage remains live
    // through the call; the driver checks its supported texture dimensions.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut result))? };
    result.context("Missing GPU scaler texture")
}
