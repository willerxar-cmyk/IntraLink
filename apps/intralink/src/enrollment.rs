//! Local enrollment UI. Never enroll a peer from a network request.
use std::{
    io::{self, BufRead, IsTerminal, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use intralink_vault::{Direction, Invitation, MAX_CODE_BYTES, Peer, Route, Vault};
use iroh::EndpointId;
use qrcode::{QrCode, render::unicode::Dense1x2};

use crate::local::{approve, prompt_password};

pub(crate) fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Invalid system clock")?
        .as_secs())
}

#[derive(Clone, Copy)]
pub(crate) enum ReviewStage {
    Issue,
    Accept,
    Confirm,
}

impl ReviewStage {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Issue => "1 / Create signed invite",
            Self::Accept => "2 / Accept signed invite",
            Self::Confirm => "3 / Confirm pairing",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Issue => "CLIENT creates an offer for this BASE",
            Self::Accept => "BASE signs a reply for this CLIENT",
            Self::Confirm => "CLIENT saves the matching inbound grant",
        }
    }

    pub(crate) fn action(self) -> &'static str {
        match self {
            Self::Issue => "Create invite",
            Self::Accept => "Sign acceptance",
            Self::Confirm => "Confirm pairing",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ReviewTime {
    New(u64),
    Expires(u64),
}

pub(crate) struct Review<'a> {
    pub stage: ReviewStage,
    pub host: EndpointId,
    pub controller: EndpointId,
    pub route: &'a Route,
    pub time: ReviewTime,
    pub name: Option<&'a str>,
}

impl Review<'_> {
    fn approve(&self, gui: bool) -> Result<()> {
        println!(
            "{}\n{}\nCLIENT identity: {}\nBASE identity: {}\nRoute: {} port {}\nScope: diagnostic pairing only; no screen/input session",
            self.stage.title(),
            self.stage.description(),
            self.host,
            self.controller,
            self.route.host(),
            self.route.port()
        );
        match self.time {
            ReviewTime::New(ttl) => {
                println!("Lifetime: {ttl} seconds; replaces any outstanding offer.")
            }
            ReviewTime::Expires(expires) => println!("Expires (Unix UTC): {expires}"),
        }
        if let Some(name) = self.name {
            println!("Local label: {name}");
        }
        if gui {
            #[cfg(all(
                feature = "desktop-preview",
                any(target_os = "windows", target_os = "linux")
            ))]
            return crate::desktop::pairing::approve(self);
            #[cfg(not(all(
                feature = "desktop-preview",
                any(target_os = "windows", target_os = "linux")
            )))]
            anyhow::bail!("Graphical pairing requires a desktop-preview build");
        }
        approve(&format!(
            "Verify both full public identities independently with the other device owner. {}?",
            self.stage.action()
        ))
    }
}

pub(crate) fn preflight(gui: bool) -> Result<()> {
    if !gui {
        return Ok(());
    }
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    {
        crate::desktop::pairing::check_available()
    }
    #[cfg(not(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    )))]
    {
        anyhow::bail!("Graphical pairing requires a desktop-preview build")
    }
}

fn read_code() -> Result<String> {
    ensure!(
        io::stdin().is_terminal(),
        "Interactive local enrollment is required"
    );
    print!("Paste the enrollment code and press Enter: ");
    io::stdout().flush()?;
    let mut bytes = Vec::new();
    // The bounded reader stops before allocating an unbounded line.
    let mut stdin = io::stdin().lock();
    let mut limited = std::io::Read::take(&mut stdin, (MAX_CODE_BYTES + 3) as u64);
    limited.read_until(b'\n', &mut bytes)?;
    ensure!(bytes.ends_with(b"\n"), "Missing newline or code too long");
    let code = std::str::from_utf8(&bytes)?.trim_end_matches(['\r', '\n']);
    ensure!(
        !code.is_empty() && code.len() <= MAX_CODE_BYTES,
        "Invalid code length"
    );
    Ok(code.to_owned())
}

/// Construct the QR before saving a grant. Display failure after saving is
/// reported as a presentation failure, never as a rolled-back operation.
pub(crate) struct CodeDisplay {
    terminal: Option<String>,
    #[cfg(all(
        feature = "desktop-preview",
        any(target_os = "windows", target_os = "linux")
    ))]
    graphical: Option<crate::desktop::pairing::CodePanel>,
}

impl CodeDisplay {
    pub(crate) fn new(code: &str, qr: bool, gui: bool) -> Result<Self> {
        ensure!(!(qr && gui), "Choose terminal QR or graphical QR, not both");
        preflight(gui)?;
        let terminal = if qr {
            Some(
                QrCode::new(code.as_bytes())?
                    .render::<Dense1x2>()
                    .module_dimensions(1, 1)
                    .build(),
            )
        } else {
            None
        };
        Ok(Self {
            terminal,
            #[cfg(all(
                feature = "desktop-preview",
                any(target_os = "windows", target_os = "linux")
            ))]
            graphical: if gui {
                Some(crate::desktop::pairing::CodePanel::new(code)?)
            } else {
                None
            },
        })
    }

    pub(crate) fn show(self, code: &str, heading: &str, detail: &str, expires: Option<u64>) {
        println!("\n{heading}\n{detail}\n{code}\n");
        if let Some(expires) = expires {
            println!("Expires (Unix UTC): {expires}");
        }
        if let Some(image) = self.terminal {
            println!(
                "{image}\nUse a light terminal background; if wrapped or unreadable, copy the text code instead."
            );
        }
        #[cfg(all(
            feature = "desktop-preview",
            any(target_os = "windows", target_os = "linux")
        ))]
        if let Some(panel) = self.graphical
            && let Err(error) = panel.show(heading, detail, expires)
        {
            eprintln!(
                "QR window unavailable: {error:#}. The exact code is printed above; saved state is unchanged."
            );
        }
    }
}

pub fn issue(
    path: &Path,
    controller: EndpointId,
    route: Route,
    ttl: u64,
    qr: bool,
    gui: bool,
) -> Result<()> {
    preflight(gui)?;
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let mut profile = vault.load(password.as_bytes())?;
    ensure!(
        profile.role().can_receive(),
        "Only CLIENT or HYBRID can issue an invitation"
    );
    ensure!(
        profile.public_id() != controller,
        "Self-enrollment is forbidden"
    );
    Review {
        stage: ReviewStage::Issue,
        host: profile.public_id(),
        controller,
        route: &route,
        time: ReviewTime::New(ttl),
        name: None,
    }
    .approve(gui)?;
    let created = now()?;
    let code = profile.create_invite(controller, route, created, ttl)?;
    let offer = Invitation::inspect_offer(&code, created)?;
    let display = CodeDisplay::new(&code, qr, gui)?;
    vault.save(&profile, password.as_bytes())?;
    drop(password);
    drop(profile);
    println!(
        "Offer saved; no inbound grant exists until local confirmation of the signed acceptance."
    );
    display.show(
        &code,
        "Signed invite",
        "CLIENT to BASE",
        Some(offer.expires()),
    );
    Ok(())
}

pub fn accept(path: &Path, name: &str, qr: bool, gui: bool) -> Result<()> {
    preflight(gui)?;
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let mut profile = vault.load(password.as_bytes())?;
    ensure!(
        profile.role().can_initiate(),
        "Only BASE or HYBRID can accept an invitation"
    );
    let code = read_code()?;
    let invite = Invitation::inspect_offer(&code, now()?)?;
    ensure!(
        invite.controller()? == profile.public_id(),
        "Offer targets a different controller"
    );
    Peer::new(
        name,
        invite.host()?,
        Direction::Outbound,
        Some(invite.route().clone()),
    )?;
    Review {
        stage: ReviewStage::Accept,
        host: invite.host()?,
        controller: invite.controller()?,
        route: invite.route(),
        time: ReviewTime::Expires(invite.expires()),
        name: Some(name),
    }
    .approve(gui)?;
    let acceptance = profile.accept_invite(name, &code, now()?)?;
    let display = CodeDisplay::new(&acceptance, qr, gui)?;
    // No successful write may use a code that expired during QR preparation.
    invite.check_time(now()?)?;
    vault.save(&profile, password.as_bytes())?;
    drop(password);
    drop(profile);
    println!(
        "Outbound grant saved. Return the signed acceptance to the host owner for final confirmation."
    );
    display.show(
        &acceptance,
        "Signed acceptance",
        "BASE to CLIENT",
        Some(invite.expires()),
    );
    Ok(())
}

pub fn confirm(path: &Path, name: &str, gui: bool) -> Result<()> {
    preflight(gui)?;
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let mut profile = vault.load(password.as_bytes())?;
    ensure!(
        profile.role().can_receive(),
        "Only CLIENT or HYBRID can confirm an invitation"
    );
    let code = read_code()?;
    let invite = Invitation::inspect_acceptance(&code, now()?)?;
    ensure!(
        invite.host()? == profile.public_id(),
        "Acceptance targets a different host"
    );
    Peer::new(name, invite.controller()?, Direction::Inbound, None)?;
    Review {
        stage: ReviewStage::Confirm,
        host: invite.host()?,
        controller: invite.controller()?,
        route: invite.route(),
        time: ReviewTime::Expires(invite.expires()),
        name: Some(name),
    }
    .approve(gui)?;
    profile.confirm_invite(name, &code, now()?)?;
    vault.save(&profile, password.as_bytes())?;
    println!("Inbound diagnostic grant saved. Invitation consumed; no listener has been opened.");
    Ok(())
}

pub fn cancel(path: &Path) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let mut profile = vault.load(password.as_bytes())?;
    approve("Cancel the outstanding local invitation? Existing grants are unchanged.")?;
    profile.cancel_invite()?;
    vault.save(&profile, password.as_bytes())?;
    println!("Outstanding invitation canceled.");
    Ok(())
}
