//! One binary for controller, host and hybrid profiles.
#![forbid(unsafe_code)]

mod ddns;
#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
mod desktop;
mod discovery;
mod enrollment;
mod journal;
mod local;
#[cfg(feature = "resource-monitor")]
mod resources;
mod routing;
#[cfg(all(feature = "vaapi-probe", target_os = "linux"))]
mod vaapi;

use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr},
    path::PathBuf,
};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use intralink_core::{
    Role,
    journal::{Operation, Outcome},
};
use intralink_vault::{Direction, Route};
use iroh::EndpointId;

#[derive(Parser)]
#[command(
    version,
    about = "IntraLink direct-only authenticated development preview"
)]
struct Cli {
    /// Local encrypted profile file. Defaults to the OS user data directory.
    #[arg(long, global = true)]
    vault: Option<PathBuf>,
    /// Opt in to bounded local metadata for an existing foreground session attempt.
    #[arg(long, global = true)]
    record_session: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProfileRole {
    Controller,
    Host,
    Hybrid,
}

impl From<ProfileRole> for Role {
    fn from(value: ProfileRole) -> Self {
        match value {
            ProfileRole::Controller => Self::Controller,
            ProfileRole::Host => Self::Host,
            ProfileRole::Hybrid => Self::Hybrid,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum GrantDirection {
    Inbound,
    Outbound,
}

#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
#[derive(Clone, Copy, ValueEnum)]
enum MediaMode {
    Jpeg,
    Tiles,
    Compact,
}

#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
impl From<MediaMode> for intralink_core::desktop::CodecMode {
    fn from(value: MediaMode) -> Self {
        match value {
            MediaMode::Jpeg => Self::Jpeg,
            MediaMode::Tiles => Self::Tiles,
            MediaMode::Compact => Self::Compact,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Sample local CPU and memory only. No vault, network or remote access.
    #[cfg(feature = "resource-monitor")]
    Resources {
        #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(500..=10000))]
        interval_ms: u64,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=120))]
        samples: u16,
        /// Emit one JSON object per sample.
        #[arg(long)]
        json: bool,
    },
    /// Query local Linux VA-API video capabilities; no network or screen capture.
    #[cfg(all(feature = "vaapi-probe", target_os = "linux"))]
    MediaCapabilities,
    /// Create a password-encrypted local identity without overwriting an existing profile.
    Init {
        #[arg(long)]
        role: ProfileRole,
    },
    /// Show the local public identity for independent verification.
    Identity,
    /// Add a locally approved diagnostic-only grant. No remote enrollment.
    Pair {
        #[arg(long)]
        name: String,
        #[arg(long)]
        peer_id: EndpointId,
        #[arg(long)]
        direction: GrantDirection,
        /// Hostname or IP address, only for an outbound grant.
        #[arg(long, requires = "port")]
        host: Option<String>,
        #[arg(long, requires = "host")]
        port: Option<u16>,
    },
    /// Issue a signed offline invitation targeting an independently verified controller.
    Invite {
        #[arg(long)]
        controller_id: EndpointId,
        #[arg(long)]
        host: String,
        #[arg(long)]
        port: u16,
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(60..=900))]
        ttl: u64,
        #[arg(long)]
        qr: bool,
    },
    /// Import a signed offer locally and return a signed acceptance code.
    AcceptInvite {
        #[arg(long)]
        name: String,
        #[arg(long)]
        qr: bool,
    },
    /// Final local host approval of the controller's signed acceptance.
    ConfirmInvite {
        #[arg(long)]
        name: String,
    },
    /// Cancel the outstanding local invitation.
    CancelInvite,
    /// List locally approved peers.
    Devices,
    /// Read the optional local session-attempt journal, without opening the vault.
    History,
    /// Remove a locally approved peer.
    Revoke {
        #[arg(long)]
        name: String,
    },
    /// Listen for one approved diagnostic session; no remote control.
    Listen {
        #[arg(long)]
        bind: SocketAddr,
        #[arg(long)]
        controller: String,
        #[arg(long)]
        advertise_lan: bool,
    },
    /// Send one diagnostic probe using a saved outbound grant.
    Connect {
        #[arg(long)]
        device: String,
        #[arg(long)]
        lan: bool,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=3))]
        attempts: u8,
    },
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
    #[cfg(all(feature = "desktop-preview", target_os = "windows"))]
    #[command(hide = true)]
    DesktopTestPattern {
        #[arg(long)]
        height: u16,
        #[arg(long)]
        static_pattern: bool,
        #[arg(long, conflicts_with = "static_pattern")]
        geometry_pattern: bool,
    },
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
        /// Explicit native capture backend. WGC requires the Windows wgc-preview feature.
        #[arg(long, value_enum, default_value = "snapshot")]
        capture: desktop::CaptureBackend,
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=3600))]
        seconds: u64,
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u32).range(1..=60))]
        fps: u32,
        /// Output height: 540, 720 or 1080. Aspect ratio is preserved.
        #[arg(long, default_value_t = 540)]
        height: u16,
        /// JPEG quality (35-90); lower values reduce bandwidth.
        #[arg(long, default_value_t = 75, value_parser = clap::value_parser!(u8).range(35..=90))]
        quality: u8,
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
        #[arg(long, required = true)]
        device: Vec<String>,
        #[arg(long, value_enum, default_value = "compact")]
        codec: MediaMode,
        /// Use a borderless window covering the selected local monitor (primary by default).
        #[arg(long)]
        fullscreen: bool,
        #[arg(long, requires = "fullscreen")]
        local_monitor: Option<u32>,
        /// Keep every active host at its authorized capture profile.
        #[arg(long)]
        all_live: bool,
    },
    /// Resolve a saved route; classify addresses and an optional owner-supplied router WAN IP.
    Diagnose {
        #[arg(long)]
        device: String,
        #[arg(long)]
        wan_ip: Option<IpAddr>,
    },
    /// Update one existing DuckDNS record using the official HTTPS API.
    Ddns {
        #[arg(long)]
        domain: String,
        /// Optional explicit address. Without it, DuckDNS detects the request's IPv4.
        #[arg(long)]
        ip: Option<IpAddr>,
        /// Explicit IPv6 address; disables IPv4 autodetection when supplied alone.
        #[arg(long)]
        ipv6: Option<Ipv6Addr>,
        /// Optional foreground IPv4 refresh interval in seconds.
        #[arg(long, value_parser = clap::value_parser!(u64).range(60..=3600))]
        interval: Option<u64>,
    },
}

// Create only the selected command's future. A single async dispatch match
// embeds the largest command state in every invocation, including --help,
// and can overflow the small default Windows main-thread stack in Debug.
#[inline(never)]
fn run_async<F, Fut>(make: F) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(Box::pin(make()))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
    if matches!(
        &cli.command,
        Command::DesktopDecoderWorker | Command::DesktopRestrictedDecoderWorker
    ) {
        if cli.record_session || cli.vault.is_some() {
            bail!("Decoder worker cannot access a vault or session journal");
        }
        return desktop::decoder_ipc::child(matches!(
            &cli.command,
            Command::DesktopRestrictedDecoderWorker
        ));
    }
    #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
    if matches!(&cli.command, Command::DesktopDecoderTokenCheck) {
        if cli.record_session || cli.vault.is_some() {
            bail!("Token diagnostic cannot access a vault or session journal");
        }
        return intralink_d3d11::verify_decoder_token_creation();
    }
    let session_command = matches!(
        &cli.command,
        Command::Listen { .. } | Command::Connect { .. }
    );
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    let session_command = session_command
        || matches!(
            &cli.command,
            Command::ShareDesktop { .. } | Command::ViewDesktop { .. }
        );
    if cli.record_session && !session_command {
        bail!("--record-session is only valid for listen, connect, share-desktop or view-desktop");
    }
    let record = cli.record_session;
    #[cfg(feature = "resource-monitor")]
    if let Command::Resources {
        interval_ms,
        samples,
        json,
    } = &cli.command
    {
        return run_async(|| resources::report(*interval_ms, *samples, *json));
    }
    #[cfg(all(feature = "vaapi-probe", target_os = "linux"))]
    if matches!(&cli.command, Command::MediaCapabilities) {
        return vaapi::report();
    }
    if let Command::Ddns {
        domain,
        ip,
        ipv6,
        interval,
    } = &cli.command
    {
        return run_async(|| ddns::update(domain, *ip, *ipv6, *interval));
    }
    let path = local::vault_path(cli.vault.as_deref())?;
    match cli.command {
        #[cfg(feature = "resource-monitor")]
        Command::Resources {
            interval_ms,
            samples,
            json,
        } => run_async(|| resources::report(interval_ms, samples, json)),
        #[cfg(all(feature = "vaapi-probe", target_os = "linux"))]
        Command::MediaCapabilities => vaapi::report(),
        Command::Init { role } => local::initialize(&path, role.into()),
        Command::Identity => local::identity(&path),
        Command::Pair {
            name,
            peer_id,
            direction,
            host,
            port,
        } => {
            let (direction, route) = match (direction, host, port) {
                (GrantDirection::Inbound, None, None) => (Direction::Inbound, None),
                (GrantDirection::Outbound, Some(host), Some(port)) => {
                    (Direction::Outbound, Some(Route::new(&host, port)?))
                }
                _ => bail!("Outbound grants require --host and --port; inbound grants forbid them"),
            };
            local::pair(&path, &name, peer_id, direction, route)
        }
        Command::Invite {
            controller_id,
            host,
            port,
            ttl,
            qr,
        } => enrollment::issue(&path, controller_id, Route::new(&host, port)?, ttl, qr),
        Command::AcceptInvite { name, qr } => enrollment::accept(&path, &name, qr),
        Command::ConfirmInvite { name } => enrollment::confirm(&path, &name),
        Command::CancelInvite => enrollment::cancel(&path),
        Command::Devices => local::devices(&path),
        Command::History => journal::show(&path),
        Command::Revoke { name } => local::revoke(&path, &name),
        Command::Listen {
            bind,
            controller,
            advertise_lan,
        } => run_async(|| {
            journal::run(&path, record, Operation::ProbeHost, async {
                local::listen(&path, &controller, bind, advertise_lan)
                    .await
                    .map(|()| Outcome::Completed)
            })
        }),
        Command::Connect {
            device,
            lan,
            attempts,
        } => run_async(|| {
            journal::run(&path, record, Operation::ProbeController, async {
                local::connect(&path, &device, lan, attempts)
                    .await
                    .map(|()| Outcome::Completed)
            })
        }),
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        Command::Displays => desktop::list_displays(),
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        Command::DesktopCheck => desktop::workspace::check_native_windows(),
        #[cfg(all(feature = "desktop-preview", target_os = "windows"))]
        Command::DesktopMediaCheck {
            height,
            samples,
            codec,
            wgc,
            gpu_scale,
        } => desktop::media_check::run(height, samples, codec.into(), wgc, gpu_scale),
        #[cfg(all(feature = "desktop-preview", target_os = "windows"))]
        Command::DesktopTestPattern {
            height,
            static_pattern,
            geometry_pattern,
        } => desktop::media_check::pattern_source(height, static_pattern, geometry_pattern),
        #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
        Command::DesktopGeometryCheck => desktop::media_check::geometry_check(),
        #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
        Command::DesktopNv12Check {
            height,
            samples,
            vendor,
            adapter_index,
        } => intralink_d3d11::verify_nv12(height, samples, &vendor, adapter_index),
        #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
        Command::DesktopVideoAdapters => intralink_d3d11::list_video_adapters(),
        #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
        Command::DesktopWgcCodecCheck {
            height,
            samples,
            adapter_index,
            present,
        } => desktop::media_check::wgc_codec_check(height, samples, adapter_index, present),
        #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
        Command::DesktopWgcLiveCheck {
            height,
            samples,
            adapter_index,
            cancel_after,
            isolated_decoder,
            restricted_decoder,
        } => run_async(|| {
            desktop::media_check::wgc_live_check(
                height,
                samples,
                adapter_index,
                cancel_after,
                isolated_decoder,
                restricted_decoder,
            )
        }),
        #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
        Command::DesktopDecoderTokenCheck => {
            bail!("Token diagnostic must use the private local entry")
        }
        #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
        Command::DesktopDecoderWorker | Command::DesktopRestrictedDecoderWorker => {
            bail!("Decoder worker must use the private IPC entry")
        }
        #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
        Command::DesktopH264Check {
            height,
            samples,
            adapter_index,
            precise_timer,
        } => run_async(|| desktop::h264_check::run(height, samples, adapter_index, precise_timer)),
        #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
        Command::DesktopNvencCheck {
            height,
            samples,
            adapter_index,
            output,
            decode,
        } => {
            intralink_d3d11::verify_nvenc(height, samples, adapter_index, output.as_deref(), decode)
        }
        #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
        Command::DesktopStaticCheck { height, gpu_scale } => {
            desktop::media_check::static_check(height, gpu_scale)
        }
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        Command::ShareDesktop {
            bind,
            controller,
            monitor,
            capture,
            seconds,
            fps,
            height,
            quality,
            codec,
        } => {
            let config = intralink_core::desktop::VideoConfig::new(height, fps, quality, seconds)
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid video settings; height must be 540, 720 or 1080")
                })?
                .with_mode(codec.into());
            run_async(|| {
                journal::run(
                    &path,
                    record,
                    Operation::ViewHost,
                    desktop::share(&path, &controller, bind, monitor, config, capture),
                )
            })
        }
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        Command::ViewDesktop { device, codec } => run_async(|| {
            journal::run(
                &path,
                record,
                Operation::ViewController,
                desktop::receive(&path, &device, codec.into()),
            )
        }),
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        Command::Workspace {
            device,
            codec,
            fullscreen,
            local_monitor,
            all_live,
        } => run_async(|| {
            desktop::workspace::run(
                &path,
                &device,
                codec.into(),
                fullscreen,
                local_monitor,
                all_live,
            )
        }),
        Command::Diagnose { device, wan_ip } => {
            run_async(|| local::diagnose(&path, &device, wan_ip))
        }
        Command::Ddns {
            domain,
            ip,
            ipv6,
            interval,
        } => run_async(|| ddns::update(&domain, ip, ipv6, interval)),
    }
}
