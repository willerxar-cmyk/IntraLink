//! Flattened desktop subcommands keep Clap's generated builders bounded.
//! All names remain top-level CLI commands; this is an internal group.
use crate::{MediaMode, desktop};
use clap::Subcommand;
use std::net::SocketAddr;

#[derive(Subcommand)]
pub(super) enum DesktopCommand {
    /// Measure software decoding and optionally inventory local Windows video adapters.
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    DesktopProfileCheck(desktop::ProfileCheckArgs),
    /// Enumerate physical displays on the local Windows/X11 desktop.
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    Displays,
    /// Check native windowed/fullscreen rendering locally; no capture, vault or network.
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    DesktopCheck,
    /// Benchmark native capture of an isolated generated test window; no network.
    #[cfg(all(feature = "desktop-preview", target_os = "windows"))]
    DesktopMediaCheck {
        #[arg(long, default_value_t = 1080)]
        height: u16,
        #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u16).range(1..=180))]
        samples: u16,
        #[arg(long, value_enum, default_value = "compact")]
        codec: MediaMode,
        /// Use persistent Windows Graphics Capture (requires wgc-preview).
        #[arg(long)]
        wgc: bool,
        /// Strict D3D11 video-processor scaling before readback; no CPU fallback.
        #[arg(long, requires = "wgc")]
        gpu_scale: bool,
    },
    /// Validate live WGC output retargeting on a generated local window.
    #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
    DesktopRetargetCheck,
    /// Verify a final native update survives idle time without CPU readback.
    #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
    DesktopStaticCheck {
        #[arg(long, default_value_t = 540)]
        height: u16,
        #[arg(long)]
        gpu_scale: bool,
    },
    /// Verify explicit NVIDIA WGC capture, NV12, NVENC and NVDEC on an owned test window.
    #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
    DesktopWgcCodecCheck {
        #[arg(long, default_value_t = 1080)]
        height: u16,
        #[arg(long, default_value_t = 60)]
        samples: u16,
        #[arg(long)]
        adapter_index: Option<u32>,
        /// Present the decoded test window through CUDA/D3D11 and verify sampled pixels.
        #[arg(long)]
        present: bool,
    },
    /// Stream an owned moving WGC test window through QUIC loopback and a native GPU preview.
    #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
    DesktopWgcLiveCheck {
        #[arg(long, default_value_t = 1080)]
        height: u16,
        #[arg(long, default_value_t = 60)]
        samples: u16,
        #[arg(long)]
        adapter_index: Option<u32>,
        /// Cancel the local diagnostic viewer after this many successful native submissions.
        #[arg(long)]
        cancel_after: Option<u16>,
        /// Run native decoding in a private supervised process.
        #[arg(long)]
        isolated_decoder: bool,
        /// Experimental low-integrity primary token; fail closed if OS/driver startup fails.
        #[arg(long, requires = "isolated_decoder")]
        restricted_decoder: bool,
    },
    #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
    #[command(hide = true)]
    DesktopDecoderWorker,
    #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
    #[command(hide = true)]
    DesktopRestrictedDecoderWorker,
    /// Verify reduced decoder token creation without starting a GPU or network worker.
    #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
    DesktopDecoderTokenCheck,
    /// Transfer native-generated H.264 access units over isolated authenticated QUIC.
    #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
    DesktopH264Check {
        #[arg(long, default_value_t = 1080)]
        height: u16,
        #[arg(long, default_value_t = 60)]
        samples: u16,
        #[arg(long)]
        adapter_index: Option<u32>,
        /// Use one private Windows high-resolution waitable timer for pacing.
        #[arg(long)]
        precise_timer: bool,
    },
    /// Verify the native NVIDIA H.264 encoder on generated GPU NV12 surfaces.
    #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
    DesktopNvencCheck {
        #[arg(long, default_value_t = 1080)]
        height: u16,
        #[arg(long, default_value_t = 60)]
        samples: u16,
        #[arg(long)]
        adapter_index: Option<u32>,
        /// Optional generated H.264 clip; existing files are never overwritten.
        #[arg(long)]
        output: Option<std::path::PathBuf>,
        /// Decode generated H.264 packets using native NVDEC and verify sampled colors.
        #[arg(long)]
        decode: bool,
    },
    /// Verify real GPU RGBA/NV12 BT.709 conversion with generated color bands.
    #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
    DesktopNv12Check {
        #[arg(long, default_value_t = 1080)]
        height: u16,
        #[arg(long, default_value_t = 12)]
        samples: u16,
        #[arg(long, default_value = "default", value_parser = ["default", "intel", "nvidia", "amd"])]
        vendor: String,
        /// Select a specific current DXGI index; vendor must match if specified.
        #[arg(long)]
        adapter_index: Option<u32>,
    },
    /// List native DXGI video adapter identities and current diagnostic indices.
    #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
    DesktopVideoAdapters,
    /// Verify rejection of a real WGC source resize and native device mismatch.
    #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
    DesktopGeometryCheck,
    /// Local synthetic test fixture, used only by desktop diagnostics.
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    #[command(hide = true)]
    DesktopTestPattern(desktop::PatternArgs),
    /// Share one locally selected display with one paired controller (read-only).
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    ShareDesktop {
        #[arg(long)]
        bind: SocketAddr,
        #[arg(long)]
        controller: String,
        #[arg(long)]
        monitor: u32,
        /// Native capture backend; auto measures an owned test window before approval.
        #[arg(long, value_enum, default_value = "snapshot")]
        capture: desktop::CaptureBackend,
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=3600))]
        seconds: u64,
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u32).range(1..=60))]
        fps: u32,
        /// Maximum output height: 540, 720 or 1080. Defaults to 1080.
        #[arg(long, default_value_t = 1080)]
        height: u16,
        /// Maximum JPEG quality (35-90); adaptive by default.
        #[arg(long, default_value_t = 75, value_parser = clap::value_parser!(u8).range(35..=90))]
        quality: u8,
        /// Keep the locally selected JPEG quality fixed instead of adapting to receipt pressure.
        #[arg(long)]
        fixed_quality: bool,
        /// Overlap two ordered capture demands (fixed v8 and fixed quality only).
        #[arg(long, requires = "fixed_quality", conflicts_with_all = ["adaptive_resolution", "negotiate_codec", "negotiate_profile"])]
        pipeline: bool,
        /// Use adaptive-resolution protocol v10; both endpoints must opt in.
        #[arg(long)]
        adaptive_resolution: bool,
        /// Negotiate mutually enabled software codecs and adaptive geometry (v11).
        /// In v11, --codec is the richest allowed mode; both peers must opt in.
        #[arg(long, conflicts_with = "adaptive_resolution")]
        negotiate_codec: bool,
        /// Negotiate bounded viewer resolution/FPS and software codecs (v12).
        #[arg(long, conflicts_with_all = ["adaptive_resolution", "negotiate_codec"])]
        negotiate_profile: bool,
        /// Explicit compact tiles, basic tiles or independent JPEG mode. Must match the viewer.
        #[arg(long, value_enum, default_value = "compact")]
        codec: MediaMode,
    },
    /// Open a view-only window to a paired host with a locally approved session.
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    ViewDesktop {
        #[arg(long)]
        device: String,
        /// Request v10 adaptive resolution; host must also opt in.
        #[arg(long)]
        adaptive_resolution: bool,
        /// Negotiate mutually enabled software codecs and adaptive geometry (v11).
        /// In v11, --codec is the richest allowed mode; both peers must opt in.
        #[arg(long, conflicts_with = "adaptive_resolution")]
        negotiate_codec: bool,
        #[command(flatten)]
        limits: desktop::ReceiveProfileArgs,
        #[arg(long, value_enum, default_value = "compact")]
        codec: MediaMode,
    },
    /// Open a view-only workspace of paired hosts, each locally authorized.
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    Workspace {
        /// Repeat --device for up to eight distinct paired hosts.
        #[arg(long, required_unless_present = "choose", conflicts_with = "choose")]
        device: Vec<String>,
        /// Choose previously paired devices in the local screen-only selector.
        #[arg(long, conflicts_with = "device_limit")]
        choose: bool,
        /// Request v10 adaptive resolution for every explicitly selected host.
        #[arg(long)]
        adaptive_resolution: bool,
        /// Negotiate mutually enabled software codecs and adaptive geometry (v11).
        /// In v11, --codec is the richest allowed mode; both peers must opt in.
        #[arg(long, conflicts_with = "adaptive_resolution")]
        negotiate_codec: bool,
        #[command(flatten)]
        limits: desktop::ReceiveProfileArgs,
        #[arg(long, value_enum, default_value = "compact")]
        codec: MediaMode,
        /// Per-selected-device ceiling NAME=HEIGHT:FPS; repeat for distinct hosts.
        #[arg(long, requires = "negotiate_profile")]
        device_limit: Vec<String>,
        /// Use a borderless window covering the selected local monitor (primary by default).
        #[arg(long)]
        fullscreen: bool,
        #[arg(long, requires = "fullscreen")]
        local_monitor: Option<u32>,
        /// Keep every active host at its authorized capture profile.
        #[arg(long)]
        all_live: bool,
    },
}
