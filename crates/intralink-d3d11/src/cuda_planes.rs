//! Copy mapped NVDEC NV12 planes directly into registered D3D11 textures.
//! NV12 itself is not registered; use supported R8 and R8G8 plane formats.
use anyhow::{Context, Result, ensure};
use libloading::os::windows::Library;
use mini_cuda::{CUDA_MEMCPY2D_st, CUarray, CUdeviceptr, CUmemorytype, CUresult, CUstream};
use std::{ffi::c_void, ptr};
use windows::{
    Win32::Graphics::{Direct3D11::*, Dxgi::Common::*},
    core::Interface,
};

use super::nvdec_driver::check;

type Resource = *mut c_void;
type Register = unsafe extern "C" fn(*mut Resource, *mut c_void, u32) -> CUresult;
type Map = unsafe extern "C" fn(u32, *mut Resource, CUstream) -> CUresult;
type Array = unsafe extern "C" fn(*mut CUarray, Resource, u32, u32) -> CUresult;
type Copy2D = unsafe extern "C" fn(*const CUDA_MEMCPY2D_st) -> CUresult;
type Unregister = unsafe extern "C" fn(Resource) -> CUresult;

struct Api {
    register: Register,
    map: Map,
    array: Array,
    copy: Copy2D,
    unmap: Map,
    unregister: Unregister,
    _library: &'static Library,
}
impl Api {
    fn load() -> Result<Self> {
        // SAFETY: fixed CUDA ABI functions loaded only from System32. The
        // module remains live until the graphics resource registrations end.
        unsafe {
            let library = crate::driver_library::cuda()?;
            Ok(Self {
                register: *library.get(b"cuGraphicsD3D11RegisterResource")?,
                map: *library.get(b"cuGraphicsMapResources")?,
                array: *library.get(b"cuGraphicsSubResourceGetMappedArray")?,
                copy: *library.get(b"cuMemcpy2D_v2")?,
                unmap: *library.get(b"cuGraphicsUnmapResources")?,
                unregister: *library.get(b"cuGraphicsUnregisterResource")?,
                _library: library,
            })
        }
    }
}

pub(super) struct Planes {
    api: Api,
    resources: [Resource; 2],
    mapped: bool,
    failed: bool,
    size: (u32, u32),
    // Retain exact registered texture objects until after unregistration.
    textures: [ID3D11Texture2D; 2],
}
impl Planes {
    pub(super) fn new(device: &ID3D11Device, size: (u32, u32)) -> Result<Self> {
        ensure!(
            matches!(size, (960, 540) | (1280, 720) | (1920, 1080)),
            "Unsupported CUDA plane size"
        );
        let textures = [
            texture(device, size, DXGI_FORMAT_R8_UNORM)?,
            texture(device, (size.0 / 2, size.1 / 2), DXGI_FORMAT_R8G8_UNORM)?,
        ];
        let mut planes = Self {
            api: Api::load()?,
            resources: [ptr::null_mut(); 2],
            mapped: false,
            failed: false,
            size,
            textures,
        };
        for index in 0..2 {
            let resource: ID3D11Resource = planes.textures[index].cast()?;
            // SAFETY: a supported, live, same-adapter D3D11 texture is
            // registered in the caller's current private CUDA context.
            check(
                // SAFETY: validated live texture in the current CUDA context.
                unsafe {
                    (planes.api.register)(&mut planes.resources[index], resource.as_raw(), 0)
                },
                "Register CUDA D3D11 plane",
            )?;
            ensure!(
                !planes.resources[index].is_null(),
                "Missing registered CUDA plane"
            );
        }
        Ok(planes)
    }

    pub(super) fn textures(&self) -> &[ID3D11Texture2D; 2] {
        &self.textures
    }

    pub(super) fn upload(&mut self, source: CUdeviceptr, pitch: u32) -> Result<()> {
        ensure!(
            !self.failed && !self.mapped && self.resources.iter().all(|p| !p.is_null()),
            "CUDA planes unavailable"
        );
        ensure!(
            source != 0 && pitch >= self.size.0 && pitch <= 65536,
            "Invalid NVDEC source pointer or pitch"
        );
        self.failed = true;
        // SAFETY: both registrations are live and not mapped. This call
        // synchronizes prior D3D11 reads before CUDA writes.
        check(
            // SAFETY: both retained registrations are live and unmapped.
            unsafe { (self.api.map)(2, self.resources.as_mut_ptr(), ptr::null_mut()) },
            "Map CUDA D3D11 planes",
        )?;
        self.mapped = true;
        let result = self.copy_planes(source, pitch);
        // SAFETY: the previous successful map must be paired with one unmap.
        // Unmap orders the CUDA writes before subsequent D3D11 rendering.
        let cleanup = check(
            unsafe { (self.api.unmap)(2, self.resources.as_mut_ptr(), ptr::null_mut()) },
            "Unmap CUDA D3D11 planes",
        );
        if cleanup.is_ok() {
            self.mapped = false;
        }
        cleanup?;
        result?;
        self.failed = false;
        Ok(())
    }

    fn copy_planes(&self, source: CUdeviceptr, pitch: u32) -> Result<()> {
        let (w, h) = (self.size.0 as usize, self.size.1 as usize);
        let chroma = source
            .checked_add(u64::from(pitch) * h as u64)
            .context("NVDEC chroma pointer overflow")?;
        for (index, (pointer, rows)) in [(source, h), (chroma, h / 2)].into_iter().enumerate() {
            let mut array: CUarray = ptr::null_mut();
            // SAFETY: mapped resources live until unmap, mip and slice zero.
            check(
                // SAFETY: registered and mapped texture, subresource zero.
                unsafe { (self.api.array)(&mut array, self.resources[index], 0, 0) },
                "Get CUDA mapped plane array",
            )?;
            ensure!(!array.is_null(), "Missing CUDA plane array");
            // Explicit construction: CUmemorytype is a Rust enum and cannot
            // be zero-initialized. UV texture has width/2 two-byte elements.
            let copy = CUDA_MEMCPY2D_st {
                srcXInBytes: 0,
                srcY: 0,
                srcMemoryType: CUmemorytype::CU_MEMORYTYPE_DEVICE,
                srcHost: ptr::null(),
                srcDevice: pointer,
                srcArray: ptr::null_mut(),
                srcPitch: pitch as usize,
                dstXInBytes: 0,
                dstY: 0,
                dstMemoryType: CUmemorytype::CU_MEMORYTYPE_ARRAY,
                dstHost: ptr::null_mut(),
                dstDevice: 0,
                dstArray: array,
                dstPitch: 0,
                WidthInBytes: w,
                Height: rows,
            };
            // SAFETY: validated dimensions and pitch, source mapped by NVDEC
            // and destination mapped by CUDA, synchronous device-to-array copy.
            check(
                // SAFETY: checked copy from live NVDEC memory to a mapped array.
                unsafe { (self.api.copy)(&copy) },
                "Copy NVDEC plane to D3D11 array",
            )?;
        }
        Ok(())
    }

    pub(super) fn close(&mut self) -> Result<()> {
        let mut error = None;
        if self.mapped {
            // SAFETY: retry cleanup only for a still-mapped registration.
            let result = check(
                unsafe { (self.api.unmap)(2, self.resources.as_mut_ptr(), ptr::null_mut()) },
                "Cleanup mapped planes",
            );
            if let Err(failure) = result {
                error = Some(failure);
            } else {
                self.mapped = false;
            }
        }
        if !self.mapped {
            for resource in &mut self.resources {
                if !resource.is_null() {
                    // SAFETY: the specific resource was registered and unmapped.
                    match check(
                        // SAFETY: live unmapped registration owned by this object.
                        unsafe { (self.api.unregister)(*resource) },
                        "Unregister CUDA D3D11 plane",
                    ) {
                        Ok(()) => *resource = ptr::null_mut(),
                        Err(failure) => {
                            if error.is_none() {
                                error = Some(failure);
                            }
                        }
                    }
                }
            }
        }
        error.map_or(Ok(()), Err)
    }
}
impl Drop for Planes {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("CUDA plane cleanup: {error:#}");
        }
    }
}

fn texture(
    device: &ID3D11Device,
    size: (u32, u32),
    format: DXGI_FORMAT,
) -> Result<ID3D11Texture2D> {
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
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        ..Default::default()
    };
    let mut texture = None;
    // SAFETY: valid descriptor and output storage; the texture is private.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    texture.context("Missing D3D11 plane texture")
}
