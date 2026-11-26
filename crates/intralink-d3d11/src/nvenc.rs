//! Optional native NVIDIA H.264 encoder. One retained NV12 surface and one
//! synchronous output buffer; no screen capture, subprocess or CPU pixel copy.
use std::{ffi::c_void, mem, ptr, slice};

use crate::readback::texture_desc_for;
use anyhow::{Context, Result, ensure};
use libloading::os::windows::Library;
use mini_nvcodec::{guids::*, nvenc_sys::*, nvenc_ver::NvencApiVersion};
use windows::{
    Win32::Graphics::{
        Direct3D11::{ID3D11Device, ID3D11Texture2D},
        Dxgi::{Common::DXGI_FORMAT_NV12, IDXGIDevice},
    },
    core::Interface,
};

const API: NvencApiVersion = NvencApiVersion::V13_1;
const MAX_PACKET: usize = 2 * 1024 * 1024;

fn status(code: NVENCSTATUS, operation: &str) -> Result<()> {
    ensure!(
        code == NVENCSTATUS::SUCCESS,
        "{operation}: NVENC status {}",
        code.0
    );
    Ok(())
}

struct Driver {
    table: NV_ENCODE_API_FUNCTION_LIST,
    // Keep all function pointers valid until after session destruction.
    _library: &'static Library,
}

impl Driver {
    fn load() -> Result<Self> {
        let library = crate::driver_library::nvenc()?;
        // SAFETY: these two exports have the pinned SDK's C ABI. The library
        // remains live, and output pointers reference initialized storage.
        let table = unsafe {
            let get = library
                .get::<PNVENCODEAPIGETMAXSUPPORTEDVERSION>(b"NvEncodeAPIGetMaxSupportedVersion")?;
            let create = library.get::<PNVENCODEAPICREATEINSTANCE>(b"NvEncodeAPICreateInstance")?;
            let mut version = 0u32;
            status(get(&mut version), "Query NVENC driver version")?;
            ensure!(
                API.supported_by(version),
                "NVENC SDK 13.1 required; driver maximum {}.{}",
                version >> 4,
                version & 15
            );
            // All fields are integers, raw pointers or nullable function pointers.
            let mut table: NV_ENCODE_API_FUNCTION_LIST = mem::zeroed();
            table.version = API.function_list_ver();
            status(create(&mut table), "Create NVENC function table")?;
            table
        };
        ensure!(
            table.nvEncOpenEncodeSessionEx.is_some()
                && table.nvEncDestroyEncoder.is_some()
                && table.nvEncGetEncodePresetConfigEx.is_some()
                && table.nvEncInitializeEncoder.is_some()
                && table.nvEncRegisterResource.is_some()
                && table.nvEncUnregisterResource.is_some()
                && table.nvEncCreateBitstreamBuffer.is_some()
                && table.nvEncDestroyBitstreamBuffer.is_some()
                && table.nvEncMapInputResource.is_some()
                && table.nvEncUnmapInputResource.is_some()
                && table.nvEncEncodePicture.is_some()
                && table.nvEncLockBitstream.is_some()
                && table.nvEncUnlockBitstream.is_some(),
            "NVENC driver function table incomplete"
        );
        Ok(Self {
            table,
            _library: library,
        })
    }
}

pub(crate) struct EncodedPicture {
    pub keyframe: bool,
    pub hardware_status: u32,
}

pub(crate) struct NativeEncoder {
    driver: Driver,
    session: *mut c_void,
    registered: NV_ENC_REGISTERED_PTR,
    bitstream: NV_ENC_OUTPUT_PTR,
    mapped: NV_ENC_INPUT_PTR,
    locked: bool,
    poisoned: bool,
    size: (u32, u32),
    sequence: u32,
    // Registration references these exact COM objects until cleanup completes.
    _device: ID3D11Device,
    input: ID3D11Texture2D,
}

impl NativeEncoder {
    pub(crate) fn new(
        device: &ID3D11Device,
        input: &ID3D11Texture2D,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self> {
        let desc = texture_desc_for(input, device, DXGI_FORMAT_NV12)?;
        let size = (desc.Width, desc.Height);
        ensure!(
            matches!(size, (960, 540) | (1280, 720) | (1920, 1080)),
            "Unsupported NVENC profile dimensions"
        );
        ensure!(
            (1..=60).contains(&fps) && (500_000..=20_000_000).contains(&bitrate),
            "Invalid NVENC profile"
        );
        let dxgi: IDXGIDevice = device.cast()?;
        // SAFETY: the live D3D11 device's adapter is queried without modifying it.
        let adapter = unsafe { dxgi.GetAdapter()?.GetDesc()? };
        ensure!(
            adapter.VendorId == 0x10de,
            "NVENC requires the NVIDIA D3D11 device; no cross-adapter fallback"
        );
        let driver = Driver::load()?;
        let mut encoder = Self {
            driver,
            session: ptr::null_mut(),
            registered: ptr::null_mut(),
            bitstream: ptr::null_mut(),
            mapped: ptr::null_mut(),
            locked: false,
            poisoned: false,
            size,
            sequence: 0,
            _device: device.clone(),
            input: input.clone(),
        };
        encoder.initialize(fps, bitrate)?;
        Ok(encoder)
    }

    fn initialize(&mut self, fps: u32, bitrate: u32) -> Result<()> {
        let t = &self.driver.table;
        // SAFETY: the pinned FFI structs use integer newtypes, nullable
        // pointers and C unions; zero initialization of reserved bytes is
        // required by NVENC. All resources are retained by this owner.
        unsafe {
            let mut open: NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS = mem::zeroed();
            open.version = API.open_encode_session_ex_params_ver();
            open.apiVersion = API.nvencapi_version();
            open.deviceType = NV_ENC_DEVICE_TYPE::DIRECTX;
            open.device = self._device.as_raw();
            status(
                t.nvEncOpenEncodeSessionEx.unwrap()(&mut open, &mut self.session),
                "Open NVIDIA DirectX encode session",
            )?;
            ensure!(!self.session.is_null(), "Missing NVENC session");

            let codec = NV_ENC_CODEC_H264_GUID;
            let preset = NV_ENC_PRESET_P1_GUID;
            let tuning = NV_ENC_TUNING_INFO::ULTRA_LOW_LATENCY;
            let mut cfg: NV_ENC_PRESET_CONFIG = mem::zeroed();
            cfg.version = API.preset_config_ver();
            cfg.presetCfg.version = API.config_ver();
            status(
                t.nvEncGetEncodePresetConfigEx.unwrap()(
                    self.session,
                    codec,
                    preset,
                    tuning,
                    &mut cfg,
                ),
                "Get H.264 low-latency preset",
            )?;
            let config = &mut cfg.presetCfg;
            config.profileGUID = NV_ENC_H264_PROFILE_HIGH_GUID;
            config.gopLength = fps;
            config.frameIntervalP = 1;
            config.frameFieldMode = NV_ENC_PARAMS_FRAME_FIELD_MODE::FRAME;
            config.rcParams.version = API.rc_params_ver();
            config.rcParams.rateControlMode = NV_ENC_PARAMS_RC_MODE::CBR;
            config.rcParams.averageBitRate = bitrate;
            config.rcParams.maxBitRate = bitrate;
            config.rcParams.vbvBufferSize = bitrate / fps * 2;
            config.rcParams.vbvInitialDelay = config.rcParams.vbvBufferSize;
            config.rcParams.set_enableLookahead(0);
            config.rcParams.lookaheadDepth = 0;
            config.rcParams.set_zeroReorderDelay(1);
            config.rcParams.multiPass = NV_ENC_MULTI_PASS::DISABLED;
            let h264 = &mut config.encodeCodecConfig.h264Config;
            h264.idrPeriod = fps;
            h264.chromaFormatIDC = 1;
            h264.inputBitDepth = NV_ENC_BIT_DEPTH::DEPTH_8;
            h264.outputBitDepth = NV_ENC_BIT_DEPTH::DEPTH_8;
            h264.set_repeatSPSPPS(1);
            h264.set_outputAUD(1);
            let vui = &mut h264.h264VUIParameters;
            vui.videoSignalTypePresentFlag = 1;
            vui.videoFormat = NV_ENC_VUI_VIDEO_FORMAT::UNSPECIFIED;
            vui.videoFullRangeFlag = 0;
            vui.colourDescriptionPresentFlag = 1;
            vui.colourPrimaries = NV_ENC_VUI_COLOR_PRIMARIES::BT709;
            vui.transferCharacteristics = NV_ENC_VUI_TRANSFER_CHARACTERISTIC::BT709;
            vui.colourMatrix = NV_ENC_VUI_MATRIX_COEFFS::BT709;
            vui.chromaSampleLocationFlag = 1;
            vui.chromaSampleLocationTop = 0;
            vui.chromaSampleLocationBot = 0;
            let mut init: NV_ENC_INITIALIZE_PARAMS = mem::zeroed();
            init.version = API.initialize_params_ver();
            init.encodeGUID = codec;
            init.presetGUID = preset;
            init.encodeWidth = self.size.0;
            init.encodeHeight = self.size.1;
            init.darWidth = self.size.0;
            init.darHeight = self.size.1;
            init.frameRateNum = fps;
            init.frameRateDen = 1;
            init.enablePTD = 1;
            init.tuningInfo = tuning;
            init.encodeConfig = config;
            status(
                t.nvEncInitializeEncoder.unwrap()(self.session, &mut init),
                "Initialize H.264 NVENC",
            )?;

            let mut resource: NV_ENC_REGISTER_RESOURCE = mem::zeroed();
            resource.version = API.register_resource_ver();
            resource.resourceType = NV_ENC_INPUT_RESOURCE_TYPE::DIRECTX;
            resource.resourceToRegister = self.input.as_raw();
            resource.width = self.size.0;
            resource.height = self.size.1;
            resource.bufferFormat = NV_ENC_BUFFER_FORMAT::NV12;
            resource.bufferUsage = NV_ENC_BUFFER_USAGE::INPUT_IMAGE;
            status(
                t.nvEncRegisterResource.unwrap()(self.session, &mut resource),
                "Register retained NV12 texture",
            )?;
            self.registered = resource.registeredResource;
            ensure!(
                !self.registered.is_null(),
                "Missing registered NV12 resource"
            );
            let mut output: NV_ENC_CREATE_BITSTREAM_BUFFER = mem::zeroed();
            output.version = API.create_bitstream_buffer_ver();
            status(
                t.nvEncCreateBitstreamBuffer.unwrap()(self.session, &mut output),
                "Create NVENC bitstream",
            )?;
            self.bitstream = output.bitstreamBuffer;
            ensure!(!self.bitstream.is_null(), "Missing NVENC bitstream");
        }
        Ok(())
    }

    pub(crate) fn encode(
        &mut self,
        input: &ID3D11Texture2D,
        output: &mut Vec<u8>,
        force_idr: bool,
    ) -> Result<EncodedPicture> {
        ensure!(
            !self.poisoned && !self.locked && self.mapped.is_null(),
            "NVENC session failed or busy"
        );
        ensure!(&self.input == input, "NVENC input texture changed");
        let sequence = self
            .sequence
            .checked_add(1)
            .context("NVENC sequence exhausted")?;
        // No new submission is accepted after any error, including delayed output.
        self.poisoned = true;
        let t = &self.driver.table;
        // SAFETY: exactly one registered texture and bitstream remain live.
        // The caller serializes this synchronous submission with D3D11 writes.
        let picture = unsafe {
            self._device.GetDeviceRemovedReason()?;
            let mut map: NV_ENC_MAP_INPUT_RESOURCE = mem::zeroed();
            map.version = API.map_input_resource_ver();
            map.registeredResource = self.registered;
            status(
                t.nvEncMapInputResource.unwrap()(self.session, &mut map),
                "Map NV12 input",
            )?;
            self.mapped = map.mappedResource;
            ensure!(
                !self.mapped.is_null() && map.mappedBufferFmt == NV_ENC_BUFFER_FORMAT::NV12,
                "Invalid NVENC mapped input"
            );
            let mut pic: NV_ENC_PIC_PARAMS = mem::zeroed();
            pic.version = API.pic_params_ver();
            pic.inputWidth = self.size.0;
            pic.inputHeight = self.size.1;
            pic.frameIdx = sequence;
            pic.inputTimeStamp = u64::from(sequence);
            pic.inputDuration = 1;
            pic.inputBuffer = self.mapped;
            pic.outputBitstream = self.bitstream;
            pic.bufferFmt = NV_ENC_BUFFER_FORMAT::NV12;
            pic.pictureStruct = NV_ENC_PIC_STRUCT::FRAME;
            if force_idr || self.sequence == 0 {
                pic.encodePicFlags = NV_ENC_PIC_FLAG_FORCEIDR | NV_ENC_PIC_FLAG_OUTPUT_SPSPPS;
            }
            status(
                t.nvEncEncodePicture.unwrap()(self.session, &mut pic),
                "Encode one H.264 frame",
            )?;
            let mut lock: NV_ENC_LOCK_BITSTREAM = mem::zeroed();
            lock.version = API.lock_bitstream_ver();
            lock.outputBitstream = self.bitstream;
            status(
                t.nvEncLockBitstream.unwrap()(self.session, &mut lock),
                "Wait for NVENC bitstream",
            )?;
            self.locked = true;
            let count = lock.bitstreamSizeInBytes as usize;
            ensure!(
                !lock.bitstreamBufferPtr.is_null() && (1..=MAX_PACKET).contains(&count),
                "Invalid or oversized NVENC bitstream"
            );
            ensure!(
                lock.outputTimeStamp == u64::from(sequence),
                "NVENC output timestamp mismatch: submitted={sequence}, timestamp={}, hardware_status={}",
                lock.outputTimeStamp,
                lock.hwEncodeStatus
            );
            // The SDK declares hwEncodeStatus as a raw u32, not NVENCSTATUS;
            // the driver reported 2 with successful EncodePicture and Lock.
            // Preserve that value as evidence; independent decoding is required
            // before qualifying this experimental encoder. Never ignore API errors.
            let hardware_status = lock.hwEncodeStatus;
            output.clear();
            output.extend_from_slice(slice::from_raw_parts(
                lock.bitstreamBufferPtr.cast::<u8>(),
                count,
            ));
            let keyframe = lock.pictureType == NV_ENC_PIC_TYPE::IDR;
            ensure!(
                !(force_idr || self.sequence == 0) || keyframe,
                "Requested IDR not produced"
            );
            status(
                t.nvEncUnlockBitstream.unwrap()(self.session, self.bitstream),
                "Unlock NVENC bitstream",
            )?;
            self.locked = false;
            status(
                t.nvEncUnmapInputResource.unwrap()(self.session, self.mapped),
                "Unmap NV12 input",
            )?;
            self.mapped = ptr::null_mut();
            self._device.GetDeviceRemovedReason()?;
            EncodedPicture {
                keyframe,
                hardware_status,
            }
        };
        self.sequence = sequence;
        self.poisoned = false;
        Ok(picture)
    }

    /// No pending output is permitted in this synchronous preview.
    pub(crate) fn finish(mut self) -> Result<()> {
        ensure!(
            !self.poisoned && !self.locked && self.mapped.is_null(),
            "Cannot flush failed NVENC session"
        );
        // SAFETY: a live, initialized session with no pending input; EOS has
        // no input resource or output buffer in synchronous mode.
        unsafe {
            let mut eos: NV_ENC_PIC_PARAMS = mem::zeroed();
            eos.version = API.pic_params_ver();
            eos.encodePicFlags = NV_ENC_PIC_FLAG_EOS;
            status(
                self.driver.table.nvEncEncodePicture.unwrap()(self.session, &mut eos),
                "Flush NVENC",
            )?;
        }
        self.release()
    }

    /// Stop without publishing or flushing any further pictures after failure.
    pub(crate) fn abort(mut self) -> Result<()> {
        self.release()
    }

    fn release(&mut self) -> Result<()> {
        if self.session.is_null() {
            return Ok(());
        }
        let t = &self.driver.table;
        let mut first_error = None;
        let mut record = |result: Result<()>| {
            if let Err(error) = result {
                eprintln!("{error:#}");
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        };
        // SAFETY: only resources acquired by this session are released;
        // the driver and COM owners remain live until after this cleanup.
        unsafe {
            if self.locked {
                record(status(
                    t.nvEncUnlockBitstream.unwrap()(self.session, self.bitstream),
                    "Cleanup unlock",
                ));
                self.locked = false;
            }
            if !self.mapped.is_null() {
                record(status(
                    t.nvEncUnmapInputResource.unwrap()(self.session, self.mapped),
                    "Cleanup unmap",
                ));
                self.mapped = ptr::null_mut();
            }
            if !self.registered.is_null() {
                record(status(
                    t.nvEncUnregisterResource.unwrap()(self.session, self.registered),
                    "Cleanup unregister",
                ));
                self.registered = ptr::null_mut();
            }
            if !self.bitstream.is_null() {
                record(status(
                    t.nvEncDestroyBitstreamBuffer.unwrap()(self.session, self.bitstream),
                    "Cleanup bitstream",
                ));
                self.bitstream = ptr::null_mut();
            }
            record(status(
                t.nvEncDestroyEncoder.unwrap()(self.session),
                "Destroy NVENC session",
            ));
            self.session = ptr::null_mut();
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for NativeEncoder {
    fn drop(&mut self) {
        if let Err(error) = self.release() {
            eprintln!("NVENC cleanup failed: {error:#}");
        }
    }
}
