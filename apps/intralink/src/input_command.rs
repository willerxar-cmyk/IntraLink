//! Shared CLI for explicitly approved input sessions.
use anyhow::{Result, ensure};
use std::{
    io::{self, IsTerminal},
    net::SocketAddr,
    path::Path,
};

#[derive(clap::Subcommand)]
pub enum Action {
    /// Accept one separately approved, private-LAN input session.
    Host {
        #[arg(long)]
        bind: SocketAddr,
        #[arg(long)]
        controller: String,
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u16).range(1..=3600))]
        seconds: u16,
        /// Advertise this separately approved input listener on the private LAN.
        #[arg(long)]
        advertise_lan: bool,
        /// Use explicit input/2 with the current primary display metadata (Windows).
        #[cfg(target_os = "windows")]
        #[arg(long)]
        display_geometry: bool,
        /// Show a local click-through arrival cue after authorized input/2 activation.
        #[cfg(target_os = "windows")]
        #[arg(long, requires = "display_geometry")]
        entry_portal: Option<EntryPortalMode>,
    },
    /// Calibrate physical BASE edges against a saved topology; read-only, no connections.
    #[cfg(target_os = "windows")]
    EdgeCheck {
        #[arg(long, required = true)]
        device: Vec<String>,
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u16).range(5..=120))]
        seconds: u16,
    },
    /// Edit measured BASE and manually confirmed CLIENT display geometry offline.
    #[cfg(target_os = "windows")]
    Topology {
        #[arg(long, required = true)]
        device: Vec<String>,
    },
    /// Open an input-only focused pad for up to eight paired hosts.
    Workspace {
        #[cfg_attr(
            target_os = "windows",
            arg(long, required_unless_present = "choose", conflicts_with = "choose")
        )]
        #[cfg_attr(target_os = "linux", arg(long, required = true))]
        device: Vec<String>,
        /// Find private-LAN input announcements for the selected paired hosts.
        #[arg(long)]
        lan: bool,
        /// Session-only UDP input listener port for every selected peer (saved host and identity unchanged).
        #[arg(long, value_parser = clap::value_parser!(u16).range(1..=65535))]
        input_port: Option<u16>,
        /// Choose paired CLIENTs graphically; includes one bounded LAN scan.
        #[cfg(target_os = "windows")]
        #[arg(long)]
        choose: bool,
        #[cfg(target_os = "windows")]
        /// Named spatial pad with saved arrangement; START or F9 arms switching.
        #[arg(long, conflicts_with = "geometry")]
        spatial: bool,
        /// Review proportional DPI-aware display topology, then use the focused geometry pad.
        #[cfg(target_os = "windows")]
        #[arg(long)]
        geometry: bool,
        /// Require authenticated input/2 geometry from every selected CLIENT; no downgrade.
        #[cfg(target_os = "windows")]
        #[arg(long, requires = "geometry")]
        detect_displays: bool,
        /// Opt-in physical BASE edge handoff using a temporary full-screen foreground surface.
        #[cfg(target_os = "windows")]
        #[arg(long, requires = "detect_displays")]
        desktop_edges: bool,
        /// Foreground-only relative Raw Input mouse motion (experimental).
        #[cfg(target_os = "windows")]
        #[arg(long, requires = "desktop_edges")]
        raw_motion: bool,
        /// Use a 640x360 physical-pixel control panel instead of covering BASE.
        #[cfg(target_os = "windows")]
        #[arg(long, requires = "raw_motion")]
        compact_control: bool,
    },
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum EntryPortalMode {
    Full,
    Reduced,
}

#[cfg(target_os = "windows")]
impl From<EntryPortalMode> for intralink_input_win::PortalStyle {
    fn from(value: EntryPortalMode) -> Self {
        match value {
            EntryPortalMode::Full => Self::Full,
            EntryPortalMode::Reduced => Self::Reduced,
        }
    }
}

impl Action {
    pub async fn run(self, path: &Path) -> Result<()> {
        ensure!(
            io::stdin().is_terminal() && io::stdout().is_terminal(),
            "Input preview requires a local interactive terminal"
        );
        match self {
            Self::Host {
                bind,
                controller,
                seconds,
                advertise_lan,
                #[cfg(target_os = "windows")]
                display_geometry,
                #[cfg(target_os = "windows")]
                entry_portal,
            } => {
                super::input_host::run(
                    path,
                    bind,
                    &controller,
                    seconds,
                    advertise_lan,
                    #[cfg(target_os = "windows")]
                    display_geometry,
                    #[cfg(target_os = "windows")]
                    entry_portal.map(Into::into),
                )
                .await
            }
            #[cfg(target_os = "windows")]
            Self::EdgeCheck { device, seconds } => {
                super::input_ui::edge_check(path, &device, seconds).await
            }
            #[cfg(target_os = "windows")]
            Self::Topology { device } => super::input_ui::configure_topology(path, &device).await,
            #[cfg(target_os = "windows")]
            Self::Workspace {
                device,
                spatial,
                choose,
                lan,
                input_port,
                geometry,
                detect_displays,
                desktop_edges,
                raw_motion,
                compact_control,
            } => {
                use super::input_ui::WorkspaceMode;
                let mode = if desktop_edges {
                    WorkspaceMode::DesktopEdges {
                        source: if compact_control {
                            super::input_ui::SourceMode::CompactRaw
                        } else if raw_motion {
                            super::input_ui::SourceMode::Raw
                        } else {
                            super::input_ui::SourceMode::Polled
                        },
                    }
                } else if geometry {
                    WorkspaceMode::Geometry { detect_displays }
                } else if spatial || choose {
                    WorkspaceMode::Spatial
                } else {
                    WorkspaceMode::Manual
                };
                super::input_ui::workspace(path, &device, choose, lan, mode, input_port).await
            }
            #[cfg(target_os = "linux")]
            Self::Workspace {
                device,
                lan,
                input_port,
            } => super::input_x11_ui::workspace(path, &device, lan, input_port).await,
        }
    }
}
