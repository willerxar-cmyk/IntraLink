//! Common consent, pinned peer, bounded input transport and release lifecycle.
use crate::discovery::{Advertisement, Protocol};
#[cfg(target_os = "windows")]
use crate::input_ui::topology_store;
use crate::local::{approve, prompt_password};
use anyhow::{Result, ensure};
use intralink_core::input::MAX_SECONDS;
#[cfg(target_os = "windows")]
use intralink_core::input::{Event, Record};
use intralink_transport::input::{Delivery, bind_input, serve_input};
#[cfg(target_os = "windows")]
use intralink_transport::input::{bind_input_geometry, serve_input_geometry};
use intralink_vault::{Direction, Vault};
#[cfg(target_os = "windows")]
use std::time::Instant;
use std::{net::SocketAddr, path::Path, time::Duration};
use tokio::{sync::mpsc, time::sleep};

const POLL: Duration = Duration::from_millis(8);

/// Only explicit private IPv4 LANs. No public bind or loopback injection.
pub(super) fn private_lan(address: SocketAddr) -> bool {
    matches!(address.ip(), std::net::IpAddr::V4(ip) if ip.is_private()) && address.port() != 0
}

#[cfg(target_os = "windows")]
struct LocalInput {
    indicator: minifb::Window,
    native: intralink_input_win::NativeInput,
}

#[cfg(target_os = "windows")]
impl LocalInput {
    fn open() -> Result<Self> {
        let indicator = super::input_ui::window(
            "IntraLink: INPUT SHARING - close or focus + ESC to STOP",
            460,
            80,
            true,
            0x00b92436,
        )?;
        ensure!(indicator.is_open(), "Local input indicator closed");
        Ok(Self {
            indicator,
            native: intralink_input_win::NativeInput::default(),
        })
    }

    fn poll(&mut self) -> Result<bool> {
        self.indicator.update();
        Ok(self.indicator.is_open()
            && !(self.indicator.is_active() && self.indicator.is_key_down(minifb::Key::Escape)))
    }

    fn apply(&mut self, record: Record) -> Result<()> {
        self.native.apply(record)
    }

    fn release_all(&mut self) -> Result<()> {
        self.native.release_all()
    }
}

#[cfg(target_os = "linux")]
type LocalInput = intralink_input_x11::NativeInput;

#[cfg(target_os = "windows")]
const CONSENT: &str = "Allow this paired controller to operate the current Windows user desktop for this session? The visible red window stops access; close it or focus it and press Escape. Do not use elevated, sensitive or unattended desktops.";
#[cfg(target_os = "linux")]
const CONSENT: &str = "Allow this paired controller to operate the current local X11 desktop for this session? Keep the red stop window fully visible. Click it or press a key in it to stop. Closing, hiding or obscuring it stops access. Only a single-monitor X11 login is supported. Do not use privileged, sensitive or unattended desktops.";

pub(super) async fn run(
    path: &Path,
    address: SocketAddr,
    controller: &str,
    seconds: u16,
    advertise_lan: bool,
    #[cfg(target_os = "windows")] display_geometry: bool,
) -> Result<()> {
    ensure!(
        private_lan(address),
        "Input preview requires an explicit private IPv4 LAN address"
    );
    ensure!(
        (1..=MAX_SECONDS).contains(&seconds),
        "Invalid input duration"
    );
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    drop(password);
    ensure!(profile.role().can_receive(), "Profile cannot receive input");
    let peer = profile.peer(controller, Direction::Inbound)?.identity()?;
    let secret = profile.secret_key();
    drop(profile);
    println!(
        "Controller: {controller} ({peer})\nInput-only session: primary display, keyboard, mouse, wheel; no screen stream."
    );
    if advertise_lan {
        println!(
            "LAN announcement exposes this public identity and private IP/port, not a grant. Only the approved BASE can authenticate."
        );
    }
    #[cfg(target_os = "windows")]
    let observed = if display_geometry {
        let observed = topology_store::observe_base()?;
        let d = observed.display;
        println!(
            "Input/2 shares the approved primary display metadata only with {controller}: {}x{}, {}% scale, {} degrees rotation. Geometry changes stop the session.",
            d.width, d.height, d.scale, d.rotation
        );
        Some(observed)
    } else {
        None
    };
    approve(CONSENT)?;
    // Native initialization and the visible local stop control must succeed
    // before a listening endpoint can be opened.
    let mut native = LocalInput::open()?;
    ensure!(native.poll()?, "Local input indicator is not ready");
    #[cfg(target_os = "windows")]
    let endpoint = if let Some(observed) = observed {
        ensure!(
            topology_store::observe_base()? == observed,
            "Primary display changed before session; approve again"
        );
        bind_input_geometry(secret, address).await?
    } else {
        bind_input(secret, address).await?
    };
    #[cfg(target_os = "linux")]
    let endpoint = bind_input(secret, address).await?;
    #[cfg(target_os = "windows")]
    let protocol = if display_geometry {
        Protocol::InputGeometry
    } else {
        Protocol::Input
    };
    #[cfg(target_os = "linux")]
    let protocol = Protocol::Input;
    let advertisement = if advertise_lan {
        match Advertisement::for_protocol(endpoint.id(), address, protocol) {
            Ok(advertisement) => Some(advertisement),
            Err(error) => {
                endpoint.close().await;
                return Err(error.context("Could not advertise approved input listener"));
            }
        }
    } else {
        None
    };
    let (sender, mut receiver) = mpsc::channel::<Delivery>(1);
    let listener = endpoint.clone();
    let mut task = tokio::spawn(async move {
        #[cfg(target_os = "windows")]
        if let Some(observed) = observed {
            return serve_input_geometry(&listener, peer, seconds, sender, observed.display).await;
        }
        serve_input(&listener, peer, seconds, sender).await
    });
    #[cfg(target_os = "windows")]
    let mut last_geometry_check = Instant::now();
    #[cfg(target_os = "windows")]
    let mut check_geometry = |force: bool| -> Result<()> {
        if let Some(observed) = observed
            && (force || last_geometry_check.elapsed() >= Duration::from_millis(500))
        {
            ensure!(
                topology_store::observe_base()? == observed,
                "Primary display changed; input session stopped"
            );
            last_geometry_check = Instant::now();
        }
        Ok(())
    };
    println!(
        "Input listener active for {controller}. Local approval is session-specific; no video."
    );
    let local = async {
        loop {
            if !native.poll()? || task.is_finished() { break; }
            #[cfg(target_os = "windows")]
            check_geometry(false)?;
            tokio::select! {
                biased;
                delivery = receiver.recv() => {
                    let Some(delivery) = delivery else { break; };
                    if !native.poll()? || task.is_finished() || delivery.accepted.is_closed() { break; }
                    #[cfg(target_os = "windows")]
                    check_geometry(delivery.record.event == Event::Activate)?;
                    if !native.poll()? || task.is_finished() || delivery.accepted.is_closed() { break; }
                    let applied = native.apply(delivery.record);
                    let failed = applied.is_err();
                    if delivery.accepted.send(applied).is_err() || failed {
                        anyhow::bail!("Input acceptance failed; closing session");
                    }
                }
                _ = sleep(POLL) => {}
            }
        }
        Ok::<(), anyhow::Error>(())
    }.await;
    // Cancel producer and discard queued deliveries before releasing owned
    // input. The vault remains locked until endpoint closure and cleanup.
    let completed = task.is_finished();
    if !completed {
        task.abort();
    }
    drop(receiver);
    let released = native.release_all();
    let joined = (&mut task).await;
    endpoint.close().await;
    if let Some(advertisement) = advertisement {
        advertisement.stop().await;
    }
    local?;
    released?;
    if completed {
        joined??;
    }
    println!("Input session closed. Owned held input released.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_preview_rejects_public_loopback_and_wildcard_routes() {
        assert!(private_lan("192.168.1.20:45100".parse().unwrap()));
        assert!(!private_lan("127.0.0.1:45100".parse().unwrap()));
        assert!(!private_lan("0.0.0.0:45100".parse().unwrap()));
        assert!(!private_lan("1.1.1.1:45100".parse().unwrap()));
        assert!(!private_lan("192.168.1.20:0".parse().unwrap()));
    }
}
