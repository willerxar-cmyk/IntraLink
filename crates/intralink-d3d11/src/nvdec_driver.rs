//! Native NVIDIA CUDA/NVDEC diagnostic context and driver tables.
//! Load only System32 libraries and keep the exact D3D11 adapter identity.
use std::{ffi::c_void, ptr};

use anyhow::{Context, Result, ensure};
use libloading::os::windows::Library;
use mini_cuda::{CUcontext, CUdevice, CUdeviceptr, CUresult};
use mini_nvcodec::cuvid_sys::*;
use windows::{
    Win32::Graphics::{Direct3D11::ID3D11Device, Dxgi::IDXGIDevice},
    core::Interface,
};

pub(super) fn check(result: CUresult, operation: &str) -> Result<()> {
    ensure!(
        result == CUresult::CUDA_SUCCESS,
        "{operation}: CUDA status {}",
        result.0
    );
    Ok(())
}

type Init = unsafe extern "C" fn(u32) -> CUresult;
type Count = unsafe extern "C" fn(*mut i32) -> CUresult;
type Device = unsafe extern "C" fn(*mut CUdevice, i32) -> CUresult;
type Luid = unsafe extern "C" fn(*mut u8, *mut u32, CUdevice) -> CUresult;
type Current = unsafe extern "C" fn(*mut CUcontext) -> CUresult;
type Create = unsafe extern "C" fn(*mut CUcontext, u32, CUdevice) -> CUresult;
type Destroy = unsafe extern "C" fn(CUcontext) -> CUresult;
pub(super) type CopyToHost = unsafe extern "C" fn(*mut c_void, CUdeviceptr, usize) -> CUresult;

pub(super) struct Cuda {
    context: CUcontext,
    destroy: Destroy,
    pub copy_to_host: CopyToHost,
    _library: &'static Library,
}

impl Cuda {
    pub(super) fn new(device: &ID3D11Device) -> Result<Self> {
        let dxgi: IDXGIDevice = device.cast()?;
        // SAFETY: live DXGI adapter descriptor, no mutable resource access.
        let desc = unsafe { dxgi.GetAdapter()?.GetDesc()? };
        ensure!(desc.VendorId == 0x10de, "NVDEC requires NVIDIA adapter");
        let mut expected = [0u8; 8];
        expected[..4].copy_from_slice(&desc.AdapterLuid.LowPart.to_le_bytes());
        expected[4..].copy_from_slice(&desc.AdapterLuid.HighPart.to_le_bytes());
        Self::new_for_luid(expected)
    }

    /// Match the exact trusted adapter identity without importing D3D11
    /// into a headless diagnostic worker.
    fn new_for_luid(expected: [u8; 8]) -> Result<Self> {
        // SAFETY: resolve fixed ABI functions from the system CUDA driver
        // only; output pointers refer to initialized locals. No existing
        // context is replaced and the selected device must match DXGI LUID.
        unsafe {
            let library = crate::driver_library::cuda()?;
            let init = *library.get::<Init>(b"cuInit")?;
            let count = *library.get::<Count>(b"cuDeviceGetCount")?;
            let get_device = *library.get::<Device>(b"cuDeviceGet")?;
            let luid = *library.get::<Luid>(b"cuDeviceGetLuid")?;
            let current = *library.get::<Current>(b"cuCtxGetCurrent")?;
            let create = *library.get::<Create>(b"cuCtxCreate_v2")?;
            let destroy = *library.get::<Destroy>(b"cuCtxDestroy_v2")?;
            let copy_to_host = *library.get::<CopyToHost>(b"cuMemcpyDtoH_v2")?;
            check(init(0), "Initialize CUDA")?;
            let mut existing = ptr::null_mut();
            check(current(&mut existing), "Query current CUDA context")?;
            ensure!(
                existing.is_null(),
                "Diagnostic thread already has a CUDA context"
            );
            let mut n = 0;
            check(count(&mut n), "Enumerate CUDA devices")?;
            ensure!((1..=64).contains(&n), "Invalid CUDA device count");
            let mut selected = None;
            for ordinal in 0..n {
                let mut dev = 0;
                check(get_device(&mut dev, ordinal), "Query CUDA device")?;
                let mut bytes = [0u8; 8];
                let mut node_mask = 0;
                check(
                    luid(bytes.as_mut_ptr(), &mut node_mask, dev),
                    "Read CUDA device LUID",
                )?;
                if bytes == expected {
                    ensure!(
                        selected.is_none() && node_mask != 0,
                        "Ambiguous CUDA adapter identity"
                    );
                    selected = Some(dev);
                }
            }
            let mut context = ptr::null_mut();
            check(
                create(
                    &mut context,
                    0,
                    selected.context("No CUDA device matches D3D11 LUID")?,
                ),
                "Create private CUDA context",
            )?;
            ensure!(!context.is_null(), "Missing CUDA context");
            Ok(Self {
                context,
                destroy,
                copy_to_host,
                _library: library,
            })
        }
    }

    pub(super) fn close(&mut self) -> Result<()> {
        if self.context.is_null() {
            return Ok(());
        }
        // SAFETY: only this diagnostic's context is destroyed, on its owner thread.
        check(
            // SAFETY: this nonnull private context belongs to this thread.
            unsafe { (self.destroy)(self.context) },
            "Destroy private CUDA context",
        )?;
        self.context = ptr::null_mut();
        Ok(())
    }
}
impl Drop for Cuda {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("{error:#}");
        }
    }
}

pub(super) struct Cuvid {
    pub caps: tcuvidGetDecoderCaps,
    pub create_decoder: tcuvidCreateDecoder,
    pub destroy_decoder: tcuvidDestroyDecoder,
    pub decode: tcuvidDecodePicture,
    pub decode_status: tcuvidGetDecodeStatus,
    pub map: tcuvidMapVideoFrame64,
    pub unmap: tcuvidUnmapVideoFrame64,
    pub create_parser: tcuvidCreateVideoParser,
    pub parse: tcuvidParseVideoData,
    pub destroy_parser: tcuvidDestroyVideoParser,
    _library: &'static Library,
}
impl Cuvid {
    pub(super) fn load() -> Result<Self> {
        // SAFETY: fixed pinned C signatures from the system driver. Retain
        // the library for the entire parser/decoder callback lifetime.
        unsafe {
            let library = crate::driver_library::cuvid()?;
            Ok(Self {
                caps: *library.get(b"cuvidGetDecoderCaps")?,
                create_decoder: *library.get(b"cuvidCreateDecoder")?,
                destroy_decoder: *library.get(b"cuvidDestroyDecoder")?,
                decode: *library.get(b"cuvidDecodePicture")?,
                decode_status: *library.get(b"cuvidGetDecodeStatus")?,
                map: *library.get(b"cuvidMapVideoFrame64")?,
                unmap: *library.get(b"cuvidUnmapVideoFrame64")?,
                create_parser: *library.get(b"cuvidCreateVideoParser")?,
                parse: *library.get(b"cuvidParseVideoData")?,
                destroy_parser: *library.get(b"cuvidDestroyVideoParser")?,
                _library: library,
            })
        }
    }
}

/// Probe the real CUDA context and NVDEC capabilities without a D3D11
/// device, GUI, parser or bitstream. Never selects a different adapter.
pub fn verify_restricted_headless_cuda(expected: [u8; 8]) -> Result<()> {
    crate::harden_decoder_worker(true)?;
    eprintln!("restricted_headless_stage=cuda_context");
    let mut cuda = Cuda::new_for_luid(expected)?;
    eprintln!("restricted_headless_stage=cuvid_load");
    let cuvid = Cuvid::load()?;
    eprintln!("restricted_headless_stage=nvdec_caps");
    // SAFETY: this initialized native capability structure is queried on
    // the private current CUDA context, with fixed H.264/NV12 parameters.
    let caps = unsafe {
        let mut caps: CUVIDDECODECAPS = std::mem::zeroed();
        caps.eCodecType = cudaVideoCodec::H264;
        caps.eChromaFormat = cudaVideoChromaFormat::YUV420;
        check((cuvid.caps)(&mut caps), "Query headless H.264 capability")?;
        caps
    };
    ensure!(
        caps.bIsSupported == 1
            && caps.nOutputFormatMask & 1 != 0
            && caps.nMaxWidth >= 1920
            && caps.nMaxHeight >= 1088
            && caps.nMaxMBCount >= 120 * 68,
        "Headless H.264 1080p NV12 capability unsupported"
    );
    eprintln!(
        "restricted_headless_caps=passed, max_width={}, max_height={}, max_mb={}",
        caps.nMaxWidth, caps.nMaxHeight, caps.nMaxMBCount
    );
    cuda.close()?;
    Ok(())
}
