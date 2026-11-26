//! GPU-only conversion and swap-chain presentation of mapped NVDEC surfaces.
//! The only CPU reads are four diagnostic RGBA pixels per presented frame.
use anyhow::{Context, Result, ensure};
use std::{
    slice,
    time::{Duration, Instant},
};
use windows::{
    Win32::Graphics::{
        Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, Fxc::D3DCompile, ID3DBlob, ID3DInclude},
        Direct3D11::*,
        Dxgi::{Common::*, *},
    },
    core::{Interface, PCSTR, s},
};

use super::{cuda_planes::Planes, gpu_window::PreviewWindow};

const SHADER: &str = r#"
Texture2D<float> luma : register(t0);
Texture2D<float2> chroma : register(t1);

float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 p = float2((id << 1) & 2, id & 2);
    return float4(p * float2(2, -2) + float2(-1, 1), 0, 1);
}

float4 ps(float4 position : SV_Position) : SV_Target {
    int2 p = int2(position.xy);
    float y = (luma.Load(int3(p, 0)) * 255.0 - 16.0) / 219.0;
    float2 c = (chroma.Load(int3(p / 2, 0)) * 255.0 - 128.0) / 224.0;
    return float4(saturate(float3(y + 1.5748 * c.y,
        y - 0.187324 * c.x - 0.468124 * c.y,
        y + 1.8556 * c.x)), 1);
}
"#;

pub(super) struct Presenter {
    planes: Planes,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    views: [Option<ID3D11ShaderResourceView>; 2],
    target: Option<ID3D11RenderTargetView>,
    back: Option<ID3D11Texture2D>,
    staging: ID3D11Texture2D,
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    raster: ID3D11RasterizerState,
    swap: Option<IDXGISwapChain1>,
    window: PreviewWindow,
    size: (u32, u32),
    closed: bool,
    presented: usize,
    max_error: u8,
    times: Vec<Duration>,
}
impl Presenter {
    pub(super) fn new(device: &ID3D11Device, size: (u32, u32)) -> Result<Self> {
        let planes = Planes::new(device, size)?;
        // SAFETY: retrieve this device's immediate context and create
        // shader/view state from the two owned, registered plane textures.
        let context = unsafe { device.GetImmediateContext()? };
        let mut views = [None, None];
        for (texture, view) in planes.textures().iter().zip(&mut views) {
            // SAFETY: texture belongs to device, default typed view is valid.
            unsafe { device.CreateShaderResourceView(texture, None, Some(view))? };
            ensure!(view.is_some(), "Missing plane shader view");
        }
        let vs = compile(s!("vs"), s!("vs_5_0"))?;
        let ps = compile(s!("ps"), s!("ps_5_0"))?;
        let mut vertex = None;
        let mut pixel = None;
        let mut raster = None;
        let raster_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            DepthClipEnable: true.into(),
            ..Default::default()
        };
        // SAFETY: the compiled blobs remain live for the shader calls.
        unsafe {
            device.CreateVertexShader(
                blob_bytes(&vs)?,
                None::<&ID3D11ClassLinkage>,
                Some(&mut vertex),
            )?;
            device.CreatePixelShader(
                blob_bytes(&ps)?,
                None::<&ID3D11ClassLinkage>,
                Some(&mut pixel),
            )?;
            device.CreateRasterizerState(&raster_desc, Some(&mut raster))?;
        }
        let window = PreviewWindow::new(size)?;
        let dxgi: IDXGIDevice = device.cast()?;
        // SAFETY: use the factory of the exact device's adapter. Window is
        // owned by this thread, and swap chain is windowed, not fullscreen.
        let factory: IDXGIFactory2 = unsafe { dxgi.GetAdapter()?.GetParent()? };
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: size.0,
            Height: size.1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            ..Default::default()
        };
        // SAFETY: validated profile, live D3D11 device and owned HWND.
        let swap = unsafe {
            let hwnd = window.hwnd()?;
            factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;
            factory.CreateSwapChainForHwnd(device, hwnd, &desc, None, None::<&IDXGIOutput>)?
        };
        // SAFETY: buffer zero belongs to this swap chain.
        let back: ID3D11Texture2D = unsafe { swap.GetBuffer(0)? };
        let mut target = None;
        let mut staging = None;
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: 4,
            Height: 1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        };
        // SAFETY: back buffer and tiny staging descriptor belong to device.
        unsafe {
            device.CreateRenderTargetView(&back, None, Some(&mut target))?;
            device.CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
        }
        Ok(Self {
            planes,
            device: device.clone(),
            context,
            views,
            target: Some(target.context("Missing swap target")?),
            back: Some(back),
            staging: staging.context("Missing four-pixel staging texture")?,
            vertex: vertex.context("Missing vertex shader")?,
            pixel: pixel.context("Missing pixel shader")?,
            raster: raster.context("Missing rasterizer")?,
            swap: Some(swap),
            window,
            size,
            closed: false,
            presented: 0,
            max_error: 0,
            times: Vec::with_capacity(180),
        })
    }

    pub(super) fn draw(&mut self, pointer: u64, pitch: u32) -> Result<()> {
        ensure!(!self.closed, "GPU preview closed");
        self.window.pump()?;
        let started = Instant::now();
        self.planes.upload(pointer, pitch)?;
        let (w, h) = self.size;
        let viewport = D3D11_VIEWPORT {
            Width: w as f32,
            Height: h as f32,
            MaxDepth: 1.0,
            ..Default::default()
        };
        // SAFETY: both CUDA resources have been unmapped. Bind only the
        // owned shader state and swap-chain target, draw a fullscreen triangle.
        unsafe {
            self.device.GetDeviceRemovedReason()?;
            self.context.IASetInputLayout(None::<&ID3D11InputLayout>);
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.RSSetState(&self.raster);
            self.context.RSSetViewports(Some(&[viewport]));
            self.context.VSSetShader(&self.vertex, None);
            self.context.PSSetShader(&self.pixel, None);
            self.context.PSSetShaderResources(0, Some(&self.views));
            self.context.OMSetRenderTargets(
                Some(std::slice::from_ref(&self.target)),
                None::<&ID3D11DepthStencilView>,
            );
            self.context.Draw(3, 0);
            // Release bindings before the next CUDA map and sample copy.
            self.context.PSSetShaderResources(0, Some(&[None, None]));
            self.context
                .OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
        }
        self.verify_pixels()?;
        // SAFETY: Present the live owned swap chain only; reject a
        // non-S_OK result rather than count a failed/occluded submission.
        let presented = unsafe {
            self.swap
                .as_ref()
                .context("Missing swap chain")?
                .Present(0, DXGI_PRESENT(0))
        };
        presented.ok()?;
        ensure!(
            presented.0 == 0,
            "DXGI present returned non-success status {presented:?}"
        );
        self.presented += 1;
        self.times.push(started.elapsed());
        Ok(())
    }

    fn verify_pixels(&mut self) -> Result<()> {
        let (w, h) = self.size;
        let points = [
            (w / 4, h / 4),
            (w * 3 / 4, h / 4),
            (w / 4, h * 3 / 4),
            (w * 3 / 4, h * 3 / 4),
        ];
        let expected = [[32, 80, 128], [64, 160, 96], [192, 96, 48], [80, 48, 144]];
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: each 1x1 source box is inside the back buffer. Copy four
        // pixels to the tiny staging surface and map only those 16 bytes.
        unsafe {
            for (index, (x, y)) in points.into_iter().enumerate() {
                let rect = D3D11_BOX {
                    left: x,
                    top: y,
                    front: 0,
                    right: x + 1,
                    bottom: y + 1,
                    back: 1,
                };
                self.context.CopySubresourceRegion(
                    &self.staging,
                    0,
                    index as u32,
                    0,
                    0,
                    self.back.as_ref().context("Missing GPU back buffer")?,
                    0,
                    Some(&rect),
                );
            }
            self.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        }
        let result = (|| {
            ensure!(
                !mapped.pData.is_null() && mapped.RowPitch >= 16,
                "Invalid GPU preview pixel map"
            );
            // SAFETY: successful read map of 4x1 RGBA8 with checked pitch.
            let bytes = unsafe { slice::from_raw_parts(mapped.pData.cast::<u8>(), 16) };
            for (i, expected) in expected.into_iter().enumerate() {
                let actual = &bytes[i * 4..i * 4 + 4];
                for (a, b) in actual[..3].iter().copied().zip(expected) {
                    self.max_error = self.max_error.max(a.abs_diff(b));
                    ensure!(
                        a.abs_diff(b) <= 16,
                        "GPU preview color mismatch: pixel={i}, actual={actual:?}, expected={expected:?}"
                    );
                }
                ensure!(actual[3] == 255, "GPU preview alpha mismatch");
            }
            Ok(())
        })();
        // SAFETY: paired with the successful map above even if checking fails.
        unsafe { self.context.Unmap(&self.staging, 0) };
        result
    }

    pub(super) fn finish(&mut self, expected: usize) -> Result<()> {
        ensure!(
            self.presented == expected && expected > 0,
            "GPU preview frame count mismatch"
        );
        self.times.sort_unstable();
        let p50 = self.times[(self.times.len() - 1) / 2].as_secs_f64() * 1000.0;
        let p95 = self.times[(self.times.len() * 95).div_ceil(100) - 1].as_secs_f64() * 1000.0;
        println!(
            "gpu_preview_check=passed, presented={}, rgb_max_error={}, diagnostic_readback_bytes={}, cuda_d3d11_copy=true, upload_shader_present_p50_ms={p50:.3}, p95_ms={p95:.3}",
            self.presented,
            self.max_error,
            self.presented * 16
        );
        self.close()
    }

    pub(super) fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        // SAFETY: stop graphics bindings before unregistering CUDA resources.
        unsafe {
            self.context.ClearState();
            self.context.Flush();
        }
        self.planes.close()?;
        self.target.take();
        self.back.take();
        self.swap.take();
        // SAFETY: all swap-chain refs owned by this presenter are now released.
        // Flush deferred D3D11 destruction before destroying the HWND.
        unsafe {
            self.context.Flush();
        }
        self.window.close()?;
        self.closed = true;
        Ok(())
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("GPU preview shutdown: {error:#}");
        }
    }
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let mut code = None;
    let mut errors = None;
    // SAFETY: static source and entry names, no includes/macros or untrusted
    // code. D3DCompile writes owned blob references into initialized locals.
    let result = unsafe {
        D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            PCSTR::null(),
            None,
            None::<&ID3DInclude>,
            entry,
            target,
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(error) = result {
        let details = errors
            .as_ref()
            .and_then(|blob| blob_bytes(blob).ok())
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        anyhow::bail!("Compile local GPU shader: {error}; {details}");
    }
    code.context("Missing shader blob")
}
fn blob_bytes(blob: &ID3DBlob) -> Result<&[u8]> {
    // SAFETY: blob owns the immutable native allocation for the borrow.
    unsafe {
        let length = blob.GetBufferSize();
        let pointer = blob.GetBufferPointer();
        ensure!(
            !pointer.is_null() && (1..=1024 * 1024).contains(&length),
            "Invalid shader blob"
        );
        Ok(slice::from_raw_parts(pointer.cast::<u8>(), length))
    }
}
