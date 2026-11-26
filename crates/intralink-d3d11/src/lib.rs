//! Narrow native D3D11 boundary for the explicitly enabled WGC diagnostic.
//! Capture authorization, network, and codec state remain outside this crate.

#[cfg(all(target_os = "windows", feature = "readback"))]
mod latest;
#[cfg(all(target_os = "windows", feature = "readback"))]
mod readback;
#[cfg(all(target_os = "windows", feature = "readback"))]
pub use latest::LatestSurface;
#[cfg(all(target_os = "windows", feature = "readback"))]
pub use readback::Readback;

#[cfg(all(target_os = "windows", feature = "readback"))]
mod scale;
#[cfg(all(target_os = "windows", feature = "readback"))]
pub use scale::GpuScaler;

#[cfg(all(target_os = "windows", feature = "readback"))]
mod fixture;
#[cfg(all(target_os = "windows", feature = "readback"))]
pub use fixture::shrink_owned_window;

#[cfg(all(target_os = "windows", feature = "readback"))]
mod validation;
#[cfg(all(target_os = "windows", feature = "readback"))]
pub use validation::verify_native_boundary;

#[cfg(all(target_os = "windows", feature = "readback"))]
mod nv12;
#[cfg(all(target_os = "windows", feature = "readback"))]
pub use nv12::{GpuNv12, Nv12Surface, verify_nv12};

#[cfg(all(target_os = "windows", feature = "readback"))]
mod adapter;

#[cfg(all(target_os = "windows", feature = "readback"))]
pub use adapter::list_video_adapters;

#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod nvenc;
#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod nvenc_check;
#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
pub use nvenc_check::{EncodedFixture, FIXTURE_BITRATE, FIXTURE_FPS, verify_nvenc};

#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod nvdec;
#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod nvdec_driver;

#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod precise_timer;
#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
pub use precise_timer::PreciseTimer;

#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod selected_wgc;
#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod wgc_codec;
#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
pub use wgc_codec::{TrustedWgcPacket, stream_wgc_fixture, verify_wgc_codec, verify_wgc_stream};

#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod cuda_planes;

#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod gpu_window;

#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod gpu_present;

#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
mod driver_library;
