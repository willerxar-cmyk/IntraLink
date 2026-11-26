//! Local native CUVID parser/decoder for generated H.264 access units.
//! No remote bitstream is accepted by this module's diagnostic caller.
use std::{
    ffi::c_void,
    mem,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    time::{Duration, Instant},
};

use super::nvdec_driver::{Cuda, Cuvid, check};
use anyhow::{Context, Result, ensure};
use mini_nvcodec::cuvid_sys::*;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

const MAX_PACKET: usize = 2 * 1024 * 1024;
const COLORS: [[u8; 3]; 6] = [
    [16, 128, 128],
    [235, 128, 128],
    [63, 102, 240],
    [173, 42, 26],
    [32, 240, 118],
    [126, 128, 128],
];

#[derive(Clone, Copy)]
pub(crate) enum FixturePattern {
    Bands,
    Quadrants,
}

impl FixturePattern {
    fn rows(self, height: usize) -> [usize; 3] {
        match self {
            Self::Bands => [height / 4, height / 2, height * 3 / 4],
            Self::Quadrants => [height / 4, height * 3 / 4, height * 3 / 4],
        }
    }

    fn columns(self) -> usize {
        match self {
            Self::Bands => COLORS.len(),
            Self::Quadrants => 2,
        }
    }

    fn expected(self, column: usize, row: usize, height: usize, phase: u32) -> [u8; 3] {
        match self {
            Self::Bands => COLORS[(column + phase as usize) % COLORS.len()],
            // BT.709 limited-range values for the four existing moving
            // diagnostic window quadrant colors. Sample away from moving bar.
            Self::Quadrants => [
                [79, 154, 105],
                [132, 110, 88],
                [113, 97, 172],
                [69, 167, 138],
            ][usize::from(row >= height / 2) * 2 + column],
        }
    }
}

// Keep the optional display sink out of the headless decoder's link path.
trait SurfaceSink {
    fn draw(&mut self, pointer: u64, pitch: u32) -> Result<()>;
    fn finish(&mut self, expected: usize) -> Result<()>;
    fn close(&mut self) -> Result<()>;
}

impl SurfaceSink for super::gpu_present::Presenter {
    fn draw(&mut self, pointer: u64, pitch: u32) -> Result<()> {
        super::gpu_present::Presenter::draw(self, pointer, pitch)
    }

    fn finish(&mut self, expected: usize) -> Result<()> {
        super::gpu_present::Presenter::finish(self, expected)
    }

    fn close(&mut self) -> Result<()> {
        super::gpu_present::Presenter::close(self)
    }
}

struct State {
    // Release graphics registrations before their owning CUDA context.
    presenter: Option<Box<dyn SurfaceSink>>,
    api: Cuvid,
    cuda: Cuda,
    decoder: CUvideodecoder,
    size: (u32, u32),
    pattern: FixturePattern,
    surfaces: u32,
    submitted: u32,
    decoded: u32,
    displayed: u32,
    max_error: u8,
    failure: Option<String>,
    scratch: Vec<u8>,
    map_times: Vec<Duration>,
}

impl State {
    fn sequence(&mut self, format: &CUVIDEOFORMAT) -> Result<i32> {
        let (w, h) = self.size;
        let area = format.display_area;
        ensure!(
            format.codec == cudaVideoCodec::H264
                && format.chroma_format == cudaVideoChromaFormat::YUV420
                && format.progressive_sequence == 1
                && format.bit_depth_luma_minus8 == 0
                && format.bit_depth_chroma_minus8 == 0,
            "Unexpected H.264 sequence format"
        );
        ensure!(
            area.left == 0
                && area.top == 0
                && area.right == w as i32
                && area.bottom == h as i32
                && format.coded_width == w.div_ceil(16) * 16
                && format.coded_height == h.div_ceil(16) * 16,
            "Decoder geometry differs from diagnostic profile"
        );
        let color = format.video_signal_description;
        ensure!(
            color.video_full_range_flag() == 0
                && color.color_primaries == 1
                && color.transfer_characteristics == 1
                && color.matrix_coefficients == 1,
            "Decoder requires limited-range BT.709"
        );
        let surfaces = u32::from(format.min_num_decode_surfaces).max(2);
        ensure!((2..=32).contains(&surfaces), "Invalid decode-surface count");
        if !self.decoder.is_null() {
            ensure!(
                self.surfaces >= surfaces,
                "Decoder surface requirement changed"
            );
            return Ok(self.surfaces as i32);
        }
        // SAFETY: initialized C structs have only integer newtypes, pointers
        // and integer fields. The current CUDA context is the owned context.
        unsafe {
            let mut caps: CUVIDDECODECAPS = mem::zeroed();
            caps.eCodecType = cudaVideoCodec::H264;
            caps.eChromaFormat = cudaVideoChromaFormat::YUV420;
            check((self.api.caps)(&mut caps), "Query H.264 decode capability")?;
            ensure!(
                caps.bIsSupported == 1
                    && caps.nOutputFormatMask & 1 != 0
                    && format.coded_width <= caps.nMaxWidth
                    && format.coded_height <= caps.nMaxHeight
                    && format.coded_width / 16 * (format.coded_height / 16) <= caps.nMaxMBCount,
                "Requested NV12 H.264 decode profile unsupported"
            );
            let mut desc: CUVIDDECODECREATEINFO = mem::zeroed();
            desc.ulWidth = format.coded_width;
            desc.ulHeight = format.coded_height;
            desc.ulNumDecodeSurfaces = surfaces;
            desc.CodecType = cudaVideoCodec::H264;
            desc.ChromaFormat = cudaVideoChromaFormat::YUV420;
            desc.ulCreationFlags = cudaVideoCreate_PreferCUVID;
            desc.ulMaxWidth = format.coded_width;
            desc.ulMaxHeight = format.coded_height;
            desc.display_area = CUVIDRECT {
                left: 0,
                top: 0,
                right: w as i16,
                bottom: h as i16,
            };
            desc.OutputFormat = cudaVideoSurfaceFormat::NV12;
            desc.DeinterlaceMode = cudaVideoDeinterlaceMode::WEAVE;
            desc.ulTargetWidth = w;
            desc.ulTargetHeight = h;
            desc.ulNumOutputSurfaces = 1;
            check(
                (self.api.create_decoder)(&mut self.decoder, &mut desc),
                "Create native NVDEC decoder",
            )?;
        }
        ensure!(!self.decoder.is_null(), "Missing NVDEC decoder");
        self.surfaces = surfaces;
        Ok(surfaces as i32)
    }

    fn picture(&mut self, picture: *mut CUVIDPICPARAMS) -> Result<i32> {
        ensure!(
            !self.decoder.is_null() && !picture.is_null(),
            "Missing decoder or picture"
        );
        // SAFETY: CUVID owns the picture for this synchronous callback.
        let p = unsafe { &*picture };
        ensure!(
            p.CurrPicIdx >= 0
                && (p.CurrPicIdx as u32) < self.surfaces
                && p.field_pic_flag == 0
                && (1..=MAX_PACKET).contains(&(p.nBitstreamDataLen as usize)),
            "Invalid decoded picture parameters"
        );
        ensure!(
            self.decoded < self.submitted,
            "Unexpected extra decode picture"
        );
        // SAFETY: pass the parser-owned full ABI picture to the bound decoder.
        check(
            // SAFETY: CUVID owns the picture until the callback returns.
            unsafe { (self.api.decode)(self.decoder, picture) },
            "Submit NVDEC picture",
        )?;
        self.decoded += 1;
        Ok(1)
    }

    fn display(&mut self, info: &CUVIDPARSERDISPINFO) -> Result<i32> {
        let expected = self
            .displayed
            .checked_add(1)
            .context("Display count overflow")?;
        ensure!(
            !self.decoder.is_null()
                && info.picture_index >= 0
                && (info.picture_index as u32) < self.surfaces
                && expected <= self.submitted
                && info.timestamp == i64::from(expected),
            "NVDEC display order or surface index mismatch"
        );
        let start = Instant::now();
        let mut pointer = 0u64;
        let mut pitch = 0u32;
        // SAFETY: progressive generated stream, valid picture index, initialized
        // processing structure and live output locals. The owner thread retains
        // the current private CUDA context throughout the callback.
        unsafe {
            let mut params: CUVIDPROCPARAMS = mem::zeroed();
            params.progressive_frame = 1;
            check(
                (self.api.map)(
                    self.decoder,
                    info.picture_index,
                    &mut pointer,
                    &mut pitch,
                    &mut params,
                ),
                "Map NVDEC surface",
            )?;
        }
        let result = self
            .validate_mapped(pointer, pitch, info.picture_index, expected - 1)
            .and_then(|()| match &mut self.presenter {
                Some(presenter) => presenter.draw(pointer, pitch),
                None => Ok(()),
            });
        // SAFETY: a successful map above requires exactly one matching unmap.
        let cleanup = check(
            unsafe { (self.api.unmap)(self.decoder, pointer) },
            "Unmap NVDEC surface",
        );
        cleanup?;
        result?;
        self.map_times.push(start.elapsed());
        self.displayed = expected;
        Ok(1)
    }

    fn validate_mapped(&mut self, pointer: u64, pitch: u32, index: i32, phase: u32) -> Result<()> {
        let (w, h) = self.size;
        ensure!(
            pointer != 0 && pitch >= w && pitch <= 65536,
            "Invalid NVDEC mapped pitch"
        );
        // SAFETY: live decoder and valid mapped picture; query actual decode status.
        let status = unsafe {
            let mut status: CUVIDGETDECODESTATUS = mem::zeroed();
            check(
                (self.api.decode_status)(self.decoder, index, &mut status),
                "Get NVDEC decode status",
            )?;
            status.decodeStatus
        };
        ensure!(
            status == cuvidDecodeStatus::SUCCESS,
            "NVDEC did not complete a clean decode: {}",
            status.0
        );
        let w = w as usize;
        let h = h as usize;
        let pitch = u64::from(pitch);
        self.scratch.resize(w * 2, 0);
        // Only sample six rows; no complete CPU frame or pixel surface readback.
        for y in self.pattern.rows(h) {
            let y_offset = (y as u64)
                .checked_mul(pitch)
                .context("Luma offset overflow")?;
            let uv_offset = ((h + y / 2) as u64)
                .checked_mul(pitch)
                .context("Chroma offset overflow")?;
            let y_ptr = pointer
                .checked_add(y_offset)
                .context("Luma pointer overflow")?;
            let uv_ptr = pointer
                .checked_add(uv_offset)
                .context("Chroma pointer overflow")?;
            // SAFETY: NVDEC mapped an NV12 surface of validated target geometry.
            // Its Y rows precede h/2 interleaved UV rows of the returned pitch.
            // Copy exactly w initialized bytes from each chosen row while mapped.
            unsafe {
                check(
                    (self.cuda.copy_to_host)(self.scratch[..w].as_mut_ptr().cast(), y_ptr, w),
                    "Sample decoded luma",
                )?;
                check(
                    (self.cuda.copy_to_host)(self.scratch[w..].as_mut_ptr().cast(), uv_ptr, w),
                    "Sample decoded chroma",
                )?;
            }
            for band in 0..self.pattern.columns() {
                let x = ((2 * band + 1) * w / (2 * self.pattern.columns())) & !1;
                let actual = [
                    self.scratch[x],
                    self.scratch[w + x],
                    self.scratch[w + x + 1],
                ];
                let expected = self.pattern.expected(band, y, h, phase);
                for (a, b) in actual.into_iter().zip(expected) {
                    self.max_error = self.max_error.max(a.abs_diff(b));
                    ensure!(
                        a.abs_diff(b) <= 12,
                        "Decoded color mismatch: phase={phase}, band={band}, actual={actual:?}, expected={expected:?}"
                    );
                }
            }
        }
        Ok(())
    }
}

// CUVID invokes these callbacks synchronously during parse. Do not unwind
// into the native driver; persist failures and refuse subsequent callbacks.
fn callback(state: *mut c_void, f: impl FnOnce(&mut State) -> Result<i32>) -> i32 {
    if state.is_null() {
        return 0;
    }
    // SAFETY: parser holds the pointer to the boxed state for its full lifetime,
    // and invokes serial callbacks on the submitting thread.
    let state = unsafe { &mut *state.cast::<State>() };
    if state.failure.is_some() {
        return 0;
    }
    match catch_unwind(AssertUnwindSafe(|| f(state))) {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            state.failure = Some(format!("{error:#}"));
            0
        }
        Err(_) => {
            state.failure = Some("Native decoder callback panicked".to_owned());
            0
        }
    }
}
unsafe extern "C" fn sequence_callback(state: *mut c_void, format: *mut CUVIDEOFORMAT) -> i32 {
    callback(state, |state| {
        ensure!(!format.is_null(), "Null CUVID sequence");
        // SAFETY: parser owns the nonnull sequence pointer until callback return.
        state.sequence(unsafe { &*format })
    })
}
unsafe extern "C" fn picture_callback(state: *mut c_void, picture: *mut CUVIDPICPARAMS) -> i32 {
    callback(state, |state| state.picture(picture))
}
unsafe extern "C" fn display_callback(state: *mut c_void, info: *mut CUVIDPARSERDISPINFO) -> i32 {
    callback(state, |state| {
        ensure!(!info.is_null(), "Null CUVID display info");
        // SAFETY: parser owns the nonnull display pointer until callback return.
        state.display(unsafe { &*info })
    })
}

pub(crate) struct Decoder {
    parser: CUvideoparser,
    state: Box<State>,
}

impl Decoder {
    pub(crate) fn new(device: &ID3D11Device, size: (u32, u32)) -> Result<Self> {
        Self::with_pattern(device, size, FixturePattern::Bands)
    }

    pub(crate) fn with_pattern(
        device: &ID3D11Device,
        size: (u32, u32),
        pattern: FixturePattern,
    ) -> Result<Self> {
        Self::validate_size(size)?;
        Self::with_cuda(Cuda::new(device)?, size, pattern)
    }

    pub(crate) fn with_luid(luid: [u8; 8], size: (u32, u32)) -> Result<Self> {
        Self::validate_size(size)?;
        Self::with_cuda(Cuda::new_for_luid(luid)?, size, FixturePattern::Bands)
    }

    fn validate_size(size: (u32, u32)) -> Result<()> {
        ensure!(
            matches!(size, (960, 540) | (1280, 720) | (1920, 1080)),
            "Unsupported decoder dimensions"
        );
        Ok(())
    }

    fn with_cuda(cuda: Cuda, size: (u32, u32), pattern: FixturePattern) -> Result<Self> {
        let api = Cuvid::load()?;
        let mut state = Box::new(State {
            presenter: None,
            api,
            cuda,
            decoder: ptr::null_mut(),
            size,
            pattern,
            surfaces: 0,
            submitted: 0,
            decoded: 0,
            displayed: 0,
            max_error: 0,
            failure: None,
            scratch: Vec::new(),
            map_times: Vec::with_capacity(180),
        });
        let mut parser = ptr::null_mut();
        // SAFETY: initialized C params, function pointers match the ABI,
        // boxed callback state remains at a stable address until parser teardown.
        unsafe {
            let mut params: CUVIDPARSERPARAMS = mem::zeroed();
            params.CodecType = cudaVideoCodec::H264;
            params.ulMaxNumDecodeSurfaces = 1;
            params.ulClockRate = 60;
            params.ulMaxDisplayDelay = 0;
            params.ulErrorThreshold = 0;
            params.pUserData = (&mut *state as *mut State).cast();
            params.pfnSequenceCallback = Some(sequence_callback);
            params.pfnDecodePicture = Some(picture_callback);
            params.pfnDisplayPicture = Some(display_callback);
            check(
                (state.api.create_parser)(&mut parser, &mut params),
                "Create native H.264 parser",
            )?;
        }
        ensure!(!parser.is_null(), "Missing CUVID parser");
        Ok(Self { parser, state })
    }

    pub(crate) fn enable_preview(&mut self, device: &ID3D11Device) -> Result<()> {
        ensure!(
            self.state.submitted == 0
                && self.state.presenter.is_none()
                && matches!(self.state.pattern, FixturePattern::Quadrants),
            "Preview requires a fresh generated quadrant fixture"
        );
        self.state.presenter = Some(Box::new(super::gpu_present::Presenter::new(
            device,
            self.state.size,
        )?));
        Ok(())
    }

    pub(crate) fn submit(&mut self, packet: &[u8]) -> Result<()> {
        ensure!(
            self.state.failure.is_none() && !self.parser.is_null(),
            "Decoder failed or closed"
        );
        ensure!(
            (1..=MAX_PACKET).contains(&packet.len()) && self.state.submitted < 180,
            "Diagnostic packet or frame budget exceeded"
        );
        self.state.submitted += 1;
        let mut packet = CUVIDSOURCEDATAPACKET {
            flags: CUVID_PKT_TIMESTAMP | CUVID_PKT_ENDOFPICTURE,
            payload_size: packet.len() as u32,
            payload: packet.as_ptr(),
            timestamp: i64::from(self.state.submitted),
        };
        self.parse(&mut packet)
    }

    /// The restricted low-latency profile promises an immediate display
    /// callback for every coded access unit. Do not return transport credit
    /// for a buffered or missing picture.
    pub(crate) fn submit_immediate(&mut self, packet: &[u8]) -> Result<()> {
        ensure!(
            self.state.submitted == self.state.displayed,
            "Unexpected delayed native picture"
        );
        self.submit(packet)?;
        ensure!(
            self.state.submitted == self.state.decoded
                && self.state.submitted == self.state.displayed,
            "Native decoder did not display the submitted picture immediately"
        );
        Ok(())
    }

    fn parse(&mut self, packet: &mut CUVIDSOURCEDATAPACKET) -> Result<()> {
        let parse = self.state.api.parse;
        // SAFETY: no Rust reference to callback state is held while the live
        // parser invokes callbacks. The packet remains live for the call.
        let result = unsafe { parse(self.parser, packet) };
        if let Some(error) = &self.state.failure {
            anyhow::bail!("NVDEC callback: {error}");
        }
        check(result, "Parse native H.264 packet")
    }

    pub(crate) fn finish(self) -> Result<()> {
        println!("{}", self.finish_summary()?);
        Ok(())
    }

    pub(crate) fn finish_summary(mut self) -> Result<String> {
        let mut eos = CUVIDSOURCEDATAPACKET {
            flags: CUVID_PKT_ENDOFSTREAM,
            payload_size: 0,
            payload: ptr::null(),
            timestamp: 0,
        };
        self.parse(&mut eos)?;
        ensure!(
            self.state.submitted != 0
                && self.state.submitted == self.state.decoded
                && self.state.submitted == self.state.displayed,
            "NVDEC frame counts mismatch: submitted={}, decoded={}, displayed={}",
            self.state.submitted,
            self.state.decoded,
            self.state.displayed
        );
        if let Some(presenter) = &mut self.state.presenter {
            presenter.finish(self.state.displayed as usize)?;
        }
        self.state.map_times.sort_unstable();
        let p50 = self.state.map_times[(self.state.map_times.len() - 1) / 2].as_secs_f64() * 1000.0;
        let summary = format!(
            "nvdec_check=passed, submitted={}, decoded={}, displayed={}, sampled_yuv_max_error={}, map_and_sample_p50_ms={p50:.3}",
            self.state.submitted, self.state.decoded, self.state.displayed, self.state.max_error
        );
        self.close()?;
        Ok(summary)
    }

    /// Cancellation never submits EOS, which could publish a delayed frame.
    pub(crate) fn abort(mut self) -> Result<()> {
        self.close()
    }

    fn close(&mut self) -> Result<()> {
        // SAFETY: destroy parser before decoder and then the private CUDA
        // context; boxed state and both native libraries remain alive.
        if !self.parser.is_null() {
            check(
                // SAFETY: parser is nonnull and its callback state is still live.
                unsafe { (self.state.api.destroy_parser)(self.parser) },
                "Destroy CUVID parser",
            )?;
            self.parser = ptr::null_mut();
        }
        if !self.state.decoder.is_null() {
            check(
                // SAFETY: decoder is nonnull and its CUDA context is current.
                unsafe { (self.state.api.destroy_decoder)(self.state.decoder) },
                "Destroy NVDEC decoder",
            )?;
            self.state.decoder = ptr::null_mut();
        }
        if let Some(mut presenter) = self.state.presenter.take() {
            presenter.close()?;
        }
        self.state.cuda.close()
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("NVDEC cleanup failed: {error:#}");
        }
    }
}
