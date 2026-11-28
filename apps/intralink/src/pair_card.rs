//! Convenience transport for public pairing fields. No implicit grants.
use anyhow::{Context, Result, ensure};
use intralink_core::{
    Role,
    pair_card::{Card, MAX_CARD_BYTES},
};
use intralink_vault::{Direction, Route, Vault};
use std::{
    io::{self, IsTerminal, Read, Write},
    net::SocketAddrV4,
    path::Path,
};

pub fn show(path: &Path, address: Option<SocketAddrV4>, gui: bool) -> Result<()> {
    crate::enrollment::preflight(gui)?;
    let vault = Vault::lock(path)?;
    let password = crate::local::prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    ensure!(
        matches!(
            (profile.role(), address),
            (Role::Host, Some(_)) | (Role::Controller, None) | (Role::Hybrid, _)
        ),
        "CLIENT requires its private IP:port; BASE needs no address"
    );
    let card =
        Card::new(&profile.public_id().to_string(), address).context("Invalid private route")?;
    let code = card.encode();
    let display = crate::enrollment::CodeDisplay::new(&code, false, gui)?;
    drop(password);
    drop(profile);
    display.show(
        &code,
        "Public connection card",
        "Unsigned / grants no access",
        None,
    );
    Ok(())
}

pub fn import(path: &Path, name: &str) -> Result<()> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "Pairing requires a local interactive terminal"
    );
    print!("Paste the other device's public connection card: ");
    io::stdout().flush()?;
    let mut bytes = Vec::new();
    let mut stdin = io::stdin().lock();
    // Read only a single bounded line, not another credential or command.
    for _ in 0..=MAX_CARD_BYTES + 1 {
        let mut byte = [0];
        if stdin.read(&mut byte)? == 0 || byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
    }
    drop(stdin);
    let line = std::str::from_utf8(&bytes)?.trim_end_matches('\r');
    let card = Card::parse(line).context("Invalid connection card")?;
    let (direction, route) = match card.route() {
        Some(address) => (
            Direction::Outbound,
            Some(Route::new(&address.ip().to_string(), address.port())?),
        ),
        None => (Direction::Inbound, None),
    };
    crate::local::pair(path, name, card.identity().parse()?, direction, route)
}
