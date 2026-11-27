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
    },
    /// Open an input-only focused pad for up to eight paired hosts.
    Workspace {
        #[arg(long, required = true)]
        device: Vec<String>,
        #[cfg(target_os = "windows")]
        /// Named spatial pad with saved arrangement; START or F9 arms switching.
        #[arg(long)]
        spatial: bool,
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
            } => super::input_host::run(path, bind, &controller, seconds).await,
            #[cfg(target_os = "windows")]
            Self::Workspace { device, spatial } => {
                super::input_ui::workspace(path, &device, spatial).await
            }
            #[cfg(target_os = "linux")]
            Self::Workspace { device } => super::input_x11_ui::workspace(path, &device).await,
        }
    }
}
