//! Local enrollment UI. Never enroll a peer from a network request.
use std::{
    io::{self, BufRead, IsTerminal, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use intralink_vault::{Invitation, MAX_CODE_BYTES, Route, Vault};
use iroh::EndpointId;
use qrcode::{QrCode, render::unicode::Dense1x2};

use crate::local::{approve, prompt_password};

fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Invalid system clock")?
        .as_secs())
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

fn qr(code: &str, requested: bool) -> Result<Option<String>> {
    if !requested {
        return Ok(None);
    }
    Ok(Some(
        QrCode::new(code.as_bytes())?
            .render::<Dense1x2>()
            .module_dimensions(1, 1)
            .build(),
    ))
}

fn display(code: &str, image: Option<String>) {
    println!("\n{code}\n");
    if let Some(image) = image {
        println!(
            "{image}\nUse a light terminal background; if wrapped or unreadable, copy the text code instead."
        );
    }
}

fn show(invite: &Invitation) -> Result<()> {
    println!(
        "Host ID: {}\nController ID: {}\nRoute: {}:{}\nExpires (Unix UTC): {}\nScope: diagnostic probe only",
        invite.host()?,
        invite.controller()?,
        invite.route().host(),
        invite.route().port(),
        invite.expires()
    );
    Ok(())
}

pub fn issue(
    path: &Path,
    controller: EndpointId,
    route: Route,
    ttl: u64,
    render_qr: bool,
) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let mut profile = vault.load(password.as_bytes())?;
    println!(
        "Local host: {}\nTarget controller: {}\nRoute: {}:{}",
        profile.public_id(),
        controller,
        route.host(),
        route.port()
    );
    approve(
        "Verify the controller's full public ID independently. Issue a new expiring diagnostic invitation? This replaces any outstanding offer.",
    )?;
    let code = profile.create_invite(controller, route, now()?, ttl)?;
    let image = qr(&code, render_qr)?;
    vault.save(&profile, password.as_bytes())?;
    println!(
        "Offer saved; no inbound grant exists until local confirmation of the signed acceptance."
    );
    display(&code, image);
    Ok(())
}

pub fn accept(path: &Path, name: &str, render_qr: bool) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let mut profile = vault.load(password.as_bytes())?;
    let code = read_code()?;
    let invite = Invitation::inspect_offer(&code, now()?)?;
    ensure!(
        invite.controller()? == profile.public_id(),
        "Offer targets a different controller"
    );
    show(&invite)?;
    approve(
        "Verify the host's full public ID independently. Save this outbound diagnostic grant and sign the acceptance?",
    )?;
    let acceptance = profile.accept_invite(name, &code, now()?)?;
    let image = qr(&acceptance, render_qr)?;
    vault.save(&profile, password.as_bytes())?;
    println!(
        "Outbound grant saved. Return the signed acceptance to the host owner for final confirmation."
    );
    display(&acceptance, image);
    Ok(())
}

pub fn confirm(path: &Path, name: &str) -> Result<()> {
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let mut profile = vault.load(password.as_bytes())?;
    let code = read_code()?;
    let invite = Invitation::inspect_acceptance(&code, now()?)?;
    ensure!(
        invite.host()? == profile.public_id(),
        "Acceptance targets a different host"
    );
    show(&invite)?;
    approve(
        "Verify the controller's full public ID independently. Confirm and consume the pending invitation?",
    )?;
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
