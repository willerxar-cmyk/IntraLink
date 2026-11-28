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
    },
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
            } => {
                super::input_host::run(
                    path,
                    bind,
                    &controller,
                    seconds,
                    advertise_lan,
                    #[cfg(target_os = "windows")]
                    display_geometry,
                )
                .await
            }
            #[cfg(target_os = "windows")]
            Self::Topology { device } => super::input_ui::configure_topology(path, &device).await,
            #[cfg(target_os = "windows")]
            Self::Workspace {
                device,
                spatial,
                choose,
                lan,
                geometry,
                detect_displays,
            } => {
                super::input_ui::workspace(
                    path,
                    &device,
                    (spatial || choose) && !geometry,
                    choose,
                    lan,
                    geometry,
                    detect_displays,
                )
                .await
            }
            #[cfg(target_os = "linux")]
            Self::Workspace { device, lan } => {
                super::input_x11_ui::workspace(path, &device, lan).await
            }
        }
    }
}
