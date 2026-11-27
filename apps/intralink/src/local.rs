//! Local operator interaction, with no remote provisioning or hidden agent.
use std::{
    io::{self, IsTerminal, Write},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use directories::ProjectDirs;
use intralink_core::Role;
use intralink_transport::{bind, bind_controller, receive_probe_with_wait, send_probe_candidates};
use intralink_vault::{Direction, Peer, Profile, Route, Vault};
use iroh::{EndpointId, SecretKey};
use zeroize::Zeroizing;

use crate::{discovery, routing};

pub fn vault_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    Ok(ProjectDirs::from("org", "IntraLink", "IntraLink")
        .context("No local data directory; specify --vault")?
        .data_local_dir()
        .join("profile.vault"))
}

pub(crate) fn prompt_password() -> Result<Zeroizing<String>> {
    ensure!(
        io::stdin().is_terminal(),
        "An interactive local terminal is required"
    );
    let password = Zeroizing::new(rpassword::prompt_password("Vault passphrase: ")?);
    ensure!(
        (12..=1024).contains(&password.len()),
        "Use a passphrase of 12-1024 bytes"
    );
    Ok(password)
}

pub(crate) fn approve(prompt: &str) -> Result<()> {
    ensure!(io::stdin().is_terminal(), "Local approval is required");
    println!("{prompt}");
    print!("Type APPROVE to confirm: ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    ensure!(answer.trim() == "APPROVE", "Not approved; no change made");
    Ok(())
}

pub fn initialize(path: &Path, role: Role) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let confirmation = Zeroizing::new(rpassword::prompt_password("Repeat vault passphrase: ")?);
    ensure!(*password == *confirmation, "Passphrases do not match");
    let profile = Profile::new(role);
    vault.initialize(&profile, password.as_bytes())?;
    println!("Created local {role:?} profile. Keep the vault and passphrase safe.");
    println!("Public identity: {}", profile.public_id());
    Ok(())
}

fn role_label(role: Role) -> &'static str {
    match role {
        Role::Controller => "BASE",
        Role::Host => "CLIENT",
        Role::Hybrid => "HYBRID (advanced)",
    }
}

fn prompt_line(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    let mut answer = String::new();
    ensure!(io::stdin().read_line(&mut answer)? != 0, "Setup canceled");
    Ok(answer.trim().to_owned())
}

fn select_role() -> Result<Role> {
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    {
        crate::desktop::choose_setup_role()
    }
    #[cfg(not(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    )))]
    {
        println!("How will you use this device?");
        println!("1. BASE - Control other devices");
        println!("2. CLIENT - Receive control");
        match prompt_line("Select 1 or 2: ")?.as_str() {
            "1" => Ok(Role::Controller),
            "2" => Ok(Role::Host),
            _ => anyhow::bail!("Setup canceled; choose BASE or CLIENT"),
        }
    }
}

/// Onboarding creates no listener or grants. Credentials are entered only
/// after the local chooser closes, through the existing no-echo terminal.
pub fn setup(path: &Path, role: Option<Role>, name: Option<&str>) -> Result<()> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "Setup requires a local interactive terminal"
    );
    ensure!(
        !path.try_exists()?,
        "A local identity already exists; use identity, set-role or rename-device"
    );
    let role = match role {
        Some(role) => role,
        None => select_role()?,
    };
    ensure!(
        role != Role::Hybrid,
        "First-run setup requires BASE or CLIENT"
    );
    let name = match name {
        Some(name) => name.to_owned(),
        None => prompt_line("Device name: ")?,
    };
    let profile = Profile::new_named(role, &name)?;
    println!(
        "Role: {} | Device: {}",
        role_label(role),
        profile.device_name().unwrap_or_default()
    );
    let password = prompt_password()?;
    let confirmation = Zeroizing::new(rpassword::prompt_password("Repeat vault passphrase: ")?);
    ensure!(*password == *confirmation, "Passphrases do not match");
    approve("Create this local identity? No peer or remote capability will be authorized.")?;
    let vault = Vault::lock(path)?;
    vault.initialize(&profile, password.as_bytes())?;
    println!("Public identity: {}", profile.public_id());
    println!("Next: use pair to approve devices independently. Run --help for available commands.");
    Ok(())
}

pub fn set_role(path: &Path, next: Role) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    let impact = profile.role_change_impact(next);
    if impact.previous == impact.next {
        println!("Role is already {}. No change made.", role_label(next));
        return Ok(());
    }
    println!("{} -> {}", role_label(impact.previous), role_label(next));
    println!("Incompatible grants to revoke: {}", impact.revoked_grants);
    println!(
        "Outstanding invitation to cancel: {}",
        impact.canceled_invite
    );
    println!("Identity and compatible grants will be retained; no new grants will be created.");
    println!("Removed grants require new pairing. Do not downgrade the vault after migration.");
    approve("Apply this local role change?")?;
    vault.change_role(password.as_bytes(), impact)?;
    println!("Role updated. Public identity: {}", profile.public_id());
    Ok(())
}

pub fn rename_device(path: &Path, name: &str) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    approve("Update this local device name? Identity and grants will not change.")?;
    vault.rename_device(password.as_bytes(), name)?;
    println!("Device name updated.");
    Ok(())
}

pub fn identity(path: &Path) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    println!(
        "Role: {} ({:?})\nDevice: {}\nPublic identity: {}",
        role_label(profile.role()),
        profile.role(),
        profile.device_name().unwrap_or("(unnamed)"),
        profile.public_id()
    );
    Ok(())
}

pub fn pair(
    path: &Path,
    name: &str,
    peer_id: EndpointId,
    direction: Direction,
    route: Option<Route>,
) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let mut profile = vault.load(password.as_bytes())?;
    let peer = Peer::new(name, peer_id, direction, route)?;
    println!(
        "Local identity: {}\nPeer identity: {}\nGrant: {:?} diagnostic probe",
        profile.public_id(),
        peer_id,
        direction
    );
    if let Some(route) = peer.route() {
        println!("Routing hint: {} port {}", route.host(), route.port());
    }
    profile.add_peer(peer)?;
    approve(
        "Compare the full peer identity independently with the device owner. Save this local grant?",
    )?;
    vault.save(&profile, password.as_bytes())?;
    println!("Local grant saved. Both endpoints must independently approve the other identity.");
    Ok(())
}

pub fn devices(path: &Path) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    if profile.peers().is_empty() {
        println!("No locally approved peers.");
    }
    for peer in profile.peers() {
        println!(
            "{} {:?} {}",
            peer.name(),
            peer.direction(),
            peer.identity()?
        );
        if let Some(route) = peer.route() {
            println!("  {} port {}", route.host(), route.port());
        }
    }
    Ok(())
}

pub fn revoke(path: &Path, name: &str) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let mut profile = vault.load(password.as_bytes())?;
    profile.revoke(name)?;
    approve("Revoke the local grant? The other endpoint is not modified.")?;
    vault.save(&profile, password.as_bytes())?;
    println!("Local grant revoked. Future diagnostic sessions require a new approval.");
    Ok(())
}

pub async fn listen(
    path: &Path,
    controller: &str,
    local: SocketAddr,
    advertise_lan: bool,
) -> Result<()> {
    ensure!(local.port() != 0, "Specify a nonzero listening port");
    if advertise_lan {
        ensure!(
            discovery::eligible_address(local),
            "LAN advertisement requires a specific private IPv4 bind address"
        );
    }
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    drop(password);
    ensure!(
        profile.role().can_receive(),
        "This profile cannot receive sessions"
    );
    let peer = profile.peer(controller, Direction::Inbound)?.identity()?;
    let secret = profile.secret_key();
    drop(profile);
    if advertise_lan {
        println!(
            "LAN discovery exposes this host's public identity, IP and port to the local network."
        );
    }
    approve(
        "Open one diagnostic session for this approved controller? Ctrl+C stops the listener.",
    )?;
    let endpoint = bind(secret, local, true).await?;
    let advertisement = if advertise_lan {
        match discovery::Advertisement::start(endpoint.id(), local) {
            Ok(advertisement) => Some(advertisement),
            Err(error) => {
                endpoint.close().await;
                return Err(error.context("Could not advertise on LAN"));
            }
        }
    } else {
        None
    };
    println!(
        "Listening on {local} for {controller} ({peer}) for at most 120 seconds. Ctrl+C stops the listener."
    );
    let result = receive_probe_with_wait(&endpoint, peer, Duration::from_secs(120)).await;
    if let Some(advertisement) = advertisement {
        advertisement.stop().await;
    }
    endpoint.close().await;
    result?;
    println!("Approved diagnostic completed; listener closed.");
    Ok(())
}

async fn probe(secret: &SecretKey, identity: EndpointId, candidates: &[SocketAddr]) -> Result<()> {
    let endpoint = bind_controller(secret.clone(), candidates).await?;
    let result = send_probe_candidates(&endpoint, identity, candidates).await;
    endpoint.close().await;
    result
}

async fn try_routes(
    secret: &SecretKey,
    identity: EndpointId,
    route: &Route,
    lan: bool,
) -> Result<()> {
    if lan {
        match discovery::find(identity).await {
            Ok(candidates) if !candidates.is_empty() => {
                println!("Trying {} untrusted LAN routing hint(s)", candidates.len());
                match probe(secret, identity, &candidates).await {
                    Ok(()) => return Ok(()),
                    Err(error) => eprintln!("LAN probe failed: {error:#}. Trying the saved route."),
                }
            }
            Ok(_) => println!("No matching LAN announcement. Trying the saved route."),
            Err(error) => eprintln!("LAN discovery failed: {error:#}. Trying the saved route."),
        }
    }
    let candidates = routing::resolve(route).await?;
    println!(
        "Trying {} direct DNS/IP candidate(s). No relay.",
        candidates.len()
    );
    probe(secret, identity, &candidates).await
}

pub async fn connect(path: &Path, device: &str, lan: bool, attempts: u8) -> Result<()> {
    ensure!((1..=3).contains(&attempts), "Expected 1-3 attempts");
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    drop(password);
    ensure!(
        profile.role().can_initiate(),
        "This profile cannot initiate sessions"
    );
    let peer = profile.peer(device, Direction::Outbound)?;
    let identity = peer.identity()?;
    let route = peer.route().context("Outbound route missing")?.clone();
    let secret = profile.secret_key();
    drop(profile);
    println!("Target: {device} ({identity})");
    let start = Instant::now();
    for attempt in 1..=attempts {
        println!("Direct connection attempt {attempt}/{attempts}");
        match try_routes(&secret, identity, &route, lan).await {
            Ok(()) => {
                println!("Authenticated diagnostic succeeded; elapsed {:?}", start.elapsed());
                return Ok(());
            }
            Err(error) if attempt == attempts => return Err(error.context("Direct authenticated session failed. Check host approval, address, UDP reachability and firewall; no relay was used")),
            Err(error) => {
                eprintln!("Attempt failed: {error:#}. Resolving routes again after a short delay.");
                tokio::time::sleep(Duration::from_secs(u64::from(attempt) * 2)).await;
            }
        }
    }
    anyhow::bail!("No connection attempt completed")
}

pub async fn diagnose(path: &Path, device: &str, wan_ip: Option<IpAddr>) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    let peer = profile.peer(device, Direction::Outbound)?;
    let route = peer.route().context("Outbound route missing")?.clone();
    let identity = peer.identity()?;
    drop(profile);
    drop(password);
    drop(vault);
    println!(
        "Saved host identity: {identity}\nRoute: {}:{}",
        route.host(),
        route.port()
    );
    if let Some(ip) = wan_ip {
        println!(
            "Owner-provided router WAN IP: {ip} ({})",
            routing::address_class(ip)
        );
        match routing::normalize_ip(ip) {
            IpAddr::V4(v4) if routing::is_shared_v4(v4) => println!(
                "The address is in RFC 6598 shared space; CGNAT is a possible upstream configuration."
            ),
            IpAddr::V4(v4) if v4.is_private() => {
                println!("A private WAN address suggests an upstream private network or NAT.")
            }
            _ => println!("The WAN address alone cannot establish reachability or NAT type."),
        }
    }
    let candidates = routing::resolve(&route).await?;
    for addr in &candidates {
        println!("  {addr} ({})", routing::address_class(addr.ip()));
    }
    if let Some(ip) = wan_ip {
        use routing::WanComparison;
        let message = match routing::compare_wan(ip, &candidates) {
            WanComparison::Match => "A selected DNS/IP candidate matches the supplied WAN address.",
            WanComparison::Different => {
                "Selected same-family candidates differ from the supplied WAN address. Check DDNS freshness; a host IPv6 address can legitimately differ from its router WAN address."
            }
            WanComparison::NoSameFamily => {
                "No selected candidate has the same address family as the supplied WAN address."
            }
            WanComparison::InvalidWan => {
                "The supplied WAN address cannot be compared as a direct destination."
            }
        };
        println!("{message} This comparison cannot prove reachability or NAT type.");
    }
    println!(
        "DNS success is not connectivity or authentication. Run an approved direct probe to test both. IPv6 also requires an allowed inbound path."
    );
    Ok(())
}

// Create only the selected command's future. A single async dispatch match
// embeds the largest command state in every invocation, including --help,
// and can overflow the small default Windows main-thread stack in Debug.
#[inline(never)]
pub(crate) fn run_async<F, Fut>(make: F) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(Box::pin(make()))
}
