//! One binary for controller, host and hybrid profiles.
#![forbid(unsafe_code)]

mod ddns;
#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
mod desktop;
#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
mod desktop_command;
#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
use desktop_command::DesktopCommand;
mod discovery;
mod enrollment;
#[cfg(any(
    all(feature = "input-preview", target_os = "windows"),
    all(feature = "input-x11-preview", target_os = "linux")
))]
mod input_command;
#[cfg(any(
    all(feature = "input-preview", target_os = "windows"),
    all(feature = "input-x11-preview", target_os = "linux")
))]
mod input_host;
#[cfg(any(
    all(feature = "input-preview", target_os = "windows"),
    all(feature = "input-x11-preview", target_os = "linux")
))]
mod input_slots;
#[cfg(all(feature = "input-preview", target_os = "windows"))]
mod input_ui;
#[cfg(all(feature = "input-x11-preview", target_os = "linux"))]
mod input_x11_ui;
mod journal;
mod local;
mod pair_card;
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
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
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
    #[value(alias = "base")]
    Controller,
    #[value(alias = "client")]
    Host,
    Hybrid,
}

#[derive(Clone, Copy, ValueEnum)]
enum SetupRole {
    Base,
    Client,
}

impl From<SetupRole> for Role {
    fn from(value: SetupRole) -> Self {
        match value {
            SetupRole::Base => Self::Controller,
            SetupRole::Client => Self::Host,
        }
    }
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
    // Clap Debug builders allocate intermediate Commands on the stack.
    // Flattening keeps the large native feature group in a separate frame.
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    #[command(flatten)]
    Desktop(DesktopCommand),
    /// Query local Linux VA-API video capabilities; no network or screen capture.
    #[cfg(all(feature = "vaapi-probe", target_os = "linux"))]
    MediaCapabilities,
    /// First-run setup: choose BASE or CLIENT and create a named encrypted identity.
    Setup {
        #[arg(long)]
        role: Option<SetupRole>,
        #[arg(long)]
        name: Option<String>,
    },
    /// Change local role, preserving identity and revoking incompatible grants.
    SetRole {
        #[arg(long)]
        role: ProfileRole,
    },
    /// Change the encrypted local device name.
    RenameDevice {
        #[arg(long)]
        name: String,
    },
    /// Create a password-encrypted local identity without overwriting an existing profile.
    Init {
        #[arg(long)]
        role: ProfileRole,
    },
    /// Show the local public identity for independent verification.
    Identity,
    /// Copy this device's public connection card (CLIENT includes private IP:port).
    ConnectionCard {
        #[arg(long)]
        address: Option<std::net::SocketAddrV4>,
        /// Show the unsigned public card as a local QR window (desktop-preview).
        #[arg(long)]
        gui: bool,
    },
    /// Paste a peer's connection card and review its full identity before pairing.
    PairCard {
        #[arg(long)]
        name: String,
    },
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
        #[arg(long, conflicts_with = "gui")]
        qr: bool,
        /// Local graphical review and signed QR window (desktop-preview).
        #[arg(long)]
        gui: bool,
    },
    /// Import a signed offer locally and return a signed acceptance code.
    AcceptInvite {
        #[arg(long)]
        name: String,
        #[arg(long, conflicts_with = "gui")]
        qr: bool,
        /// Local graphical review and signed acceptance QR (desktop-preview).
        #[arg(long)]
        gui: bool,
    },
    /// Final local host approval of the controller's signed acceptance.
    ConfirmInvite {
        #[arg(long)]
        name: String,
        /// Local graphical review before final approval (desktop-preview).
        #[arg(long)]
        gui: bool,
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
    /// Explicit foreground input-only sharing (experimental software KVM).
    #[cfg(any(
        all(feature = "input-preview", target_os = "windows"),
        all(feature = "input-x11-preview", target_os = "linux")
    ))]
    Input {
        #[command(subcommand)]
        action: input_command::Action,
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

use local::run_async;

fn main() -> Result<()> {
    if std::env::args_os().len() == 1 {
        use std::io::IsTerminal;
        let path = local::vault_path(None)?;
        if !path.try_exists()? && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
        {
            return local::setup(&path, None, None);
        }
        Cli::command().print_help()?;
        println!();
        return Ok(());
    }
    let cli = Cli::parse();
    #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
    if matches!(
        &cli.command,
        Command::Desktop(DesktopCommand::DesktopDecoderWorker)
            | Command::Desktop(DesktopCommand::DesktopRestrictedDecoderWorker)
    ) {
        if cli.record_session || cli.vault.is_some() {
            bail!("Decoder worker cannot access a vault or session journal");
        }
        return desktop::decoder_ipc::child(matches!(
            &cli.command,
            Command::Desktop(DesktopCommand::DesktopRestrictedDecoderWorker)
        ));
    }
    #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
    if matches!(
        &cli.command,
        Command::Desktop(DesktopCommand::DesktopDecoderTokenCheck)
    ) {
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
            Command::Desktop(DesktopCommand::ShareDesktop { .. })
                | Command::Desktop(DesktopCommand::ViewDesktop { .. })
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
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    if let Command::Desktop(DesktopCommand::DesktopProfileCheck(args)) = cli.command {
        return args.run();
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
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        Command::Desktop(DesktopCommand::DesktopProfileCheck(args)) => args.run(),
        Command::Setup { role, name } => local::setup(&path, role.map(Into::into), name.as_deref()),
        Command::SetRole { role } => local::set_role(&path, role.into()),
        Command::RenameDevice { name } => local::rename_device(&path, &name),
        Command::Init { role } => local::initialize(&path, role.into()),
        Command::Identity => local::identity(&path),
        Command::ConnectionCard { address, gui } => pair_card::show(&path, address, gui),
        Command::PairCard { name } => pair_card::import(&path, &name),
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
            gui,
        } => enrollment::issue(&path, controller_id, Route::new(&host, port)?, ttl, qr, gui),
        Command::AcceptInvite { name, qr, gui } => enrollment::accept(&path, &name, qr, gui),
        Command::ConfirmInvite { name, gui } => enrollment::confirm(&path, &name, gui),
        Command::CancelInvite => enrollment::cancel(&path),
        Command::Devices => local::devices(&path),
        #[cfg(any(
            all(feature = "input-preview", target_os = "windows"),
            all(feature = "input-x11-preview", target_os = "linux")
        ))]
        Command::Input { action } => run_async(|| action.run(&path)),
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
        Command::Desktop(DesktopCommand::Displays) => desktop::list_displays(),
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        Command::Desktop(DesktopCommand::DesktopCheck) => {
            desktop::workspace::check_native_windows()
        }
        #[cfg(all(feature = "desktop-preview", target_os = "windows"))]
        Command::Desktop(DesktopCommand::DesktopMediaCheck {
            height,
            samples,
            codec,
            wgc,
            gpu_scale,
        }) => desktop::media_check::run(height, samples, codec.into(), wgc, gpu_scale),
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        Command::Desktop(DesktopCommand::DesktopTestPattern(args)) => args.run(),
        #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
        Command::Desktop(DesktopCommand::DesktopGeometryCheck) => {
            desktop::media_check::geometry_check()
        }
        #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
        Command::Desktop(DesktopCommand::DesktopRetargetCheck) => {
            desktop::media_check::retarget_check()
        }
        #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
        Command::Desktop(DesktopCommand::DesktopNv12Check {
            height,
            samples,
            vendor,
            adapter_index,
        }) => intralink_d3d11::verify_nv12(height, samples, &vendor, adapter_index),
        #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
        Command::Desktop(DesktopCommand::DesktopVideoAdapters) => {
            intralink_d3d11::list_video_adapters()
        }
        #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
        Command::Desktop(DesktopCommand::DesktopWgcCodecCheck {
            height,
            samples,
            adapter_index,
            present,
        }) => desktop::media_check::wgc_codec_check(height, samples, adapter_index, present),
        #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
        Command::Desktop(DesktopCommand::DesktopWgcLiveCheck {
            height,
            samples,
            adapter_index,
            cancel_after,
            isolated_decoder,
            restricted_decoder,
        }) => run_async(|| {
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
        Command::Desktop(DesktopCommand::DesktopDecoderTokenCheck) => {
            bail!("Token diagnostic must use the private local entry")
        }
        #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
        Command::Desktop(DesktopCommand::DesktopDecoderWorker)
        | Command::Desktop(DesktopCommand::DesktopRestrictedDecoderWorker) => {
            bail!("Decoder worker must use the private IPC entry")
        }
        #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
        Command::Desktop(DesktopCommand::DesktopH264Check {
            height,
            samples,
            adapter_index,
            precise_timer,
        }) => run_async(|| desktop::h264_check::run(height, samples, adapter_index, precise_timer)),
        #[cfg(all(feature = "nvenc-preview", target_os = "windows"))]
        Command::Desktop(DesktopCommand::DesktopNvencCheck {
            height,
            samples,
            adapter_index,
            output,
            decode,
        }) => {
            intralink_d3d11::verify_nvenc(height, samples, adapter_index, output.as_deref(), decode)
        }
        #[cfg(all(feature = "wgc-preview", target_os = "windows"))]
        Command::Desktop(DesktopCommand::DesktopStaticCheck { height, gpu_scale }) => {
            desktop::media_check::static_check(height, gpu_scale)
        }
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        Command::Desktop(DesktopCommand::ShareDesktop {
            bind,
            controller,
            monitor,
            capture,
            seconds,
            fps,
            height,
            quality,
            fixed_quality,
            pipeline,
            adaptive_resolution,
            negotiate_codec,
            negotiate_profile,
            codec,
        }) => {
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
                    desktop::share(
                        &path,
                        &controller,
                        bind,
                        monitor,
                        config,
                        capture,
                        desktop::StreamOptions {
                            fixed_quality,
                            pipeline,
                            protocol: desktop::view_protocol(
                                adaptive_resolution,
                                negotiate_codec,
                                negotiate_profile,
                            ),
                        },
                    ),
                )
            })
        }
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        Command::Desktop(DesktopCommand::ViewDesktop {
            device,
            codec,
            adaptive_resolution,
            negotiate_codec,
            limits,
        }) => {
            let mode = limits.single_mode(codec.into(), adaptive_resolution, negotiate_codec)?;
            run_async(|| {
                journal::run(
                    &path,
                    record,
                    Operation::ViewController,
                    desktop::receive(&path, &device, mode),
                )
            })
        }
        #[cfg(all(target_os = "windows", feature = "input-preview"))]
        Command::Desktop(DesktopCommand::ControlDesktop {
            device,
            choose,
            input_port,
            adaptive_resolution,
            negotiate_codec,
            limits,
            codec,
        }) => {
            let mode = limits.single_mode(codec.into(), adaptive_resolution, negotiate_codec)?;
            run_async(|| desktop::control::run(&path, device, choose, input_port, mode))
        }
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        Command::Desktop(DesktopCommand::Workspace {
            device,
            choose,
            codec,
            adaptive_resolution,
            negotiate_codec,
            limits,
            device_limit,
            fullscreen,
            local_monitor,
            all_live,
        }) => {
            let selected = if choose {
                desktop::workspace::Targets::Choose {
                    limits,
                    codec: codec.into(),
                    adaptive: adaptive_resolution,
                    negotiated: negotiate_codec,
                }
            } else {
                let modes = limits.workspace_modes(
                    codec.into(),
                    adaptive_resolution,
                    negotiate_codec,
                    &device,
                    &device_limit,
                )?;
                desktop::workspace::Targets::Explicit(device, modes)
            };
            run_async(|| {
                desktop::workspace::run(&path, selected, fullscreen, local_monitor, all_live)
            })
        }
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
