//! Actual OS file locks across distinct processes. No network or native input.
use crate::{Profile, SessionKind, SessionVault, Vault};
use anyhow::{Context, Result, ensure};
use intralink_core::Role;
use std::{path::Path, process::Command};

const PASSWORD: &[u8] = b"generated test-only passphrase";
const CHILD_PATH: &str = "INTRALINK_TEST_SESSION_VAULT_PATH";
const CHILD_KIND: &str = "INTRALINK_TEST_SESSION_VAULT_KIND";
const CHILD_ALLOWED: &str = "INTRALINK_TEST_SESSION_VAULT_ALLOWED";

#[test]
fn child_lock_probe() -> Result<()> {
    let Some(path) = std::env::var_os(CHILD_PATH) else {
        return Ok(());
    };
    let kind = std::env::var(CHILD_KIND)?;
    let expected = std::env::var(CHILD_ALLOWED)? == "yes";
    let path = Path::new(&path);
    let allowed = match kind.as_str() {
        "screen" => SessionVault::lock(path, SessionKind::Screen).is_ok(),
        "input" => SessionVault::lock(path, SessionKind::Input).is_ok(),
        "exclusive" => Vault::lock(path).is_ok(),
        _ => anyhow::bail!("Unknown lock fixture kind"),
    };
    ensure!(
        allowed == expected,
        "Unexpected cross-process {kind} lock outcome"
    );
    Ok(())
}

fn probe(path: &Path, kind: &str, allowed: bool) -> Result<()> {
    let output = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "session_lease_tests::child_lock_probe",
            "--nocapture",
        ])
        .env(CHILD_PATH, path)
        .env(CHILD_KIND, kind)
        .env(CHILD_ALLOWED, if allowed { "yes" } else { "no" })
        .output()?;
    ensure!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "Child {kind} lock probe failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

#[test]
fn same_profile_allows_exactly_one_screen_and_one_input_lease() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("profile.vault");
    let profile = Profile::new_named(Role::Host, "Fixture")?;
    let id = profile.public_id();
    {
        let exclusive = Vault::lock(&path)?;
        exclusive.initialize(&profile, PASSWORD)?;
        probe(&path, "screen", false)?;
        probe(&path, "input", false)?;
    }
    let original = std::fs::read(&path)?;
    let screen = SessionVault::lock(&path, SessionKind::Screen)?;
    probe(&path, "input", true)?;
    probe(&path, "screen", false)?;
    probe(&path, "exclusive", false)?;
    let input = SessionVault::lock(&path, SessionKind::Input)?;
    assert_eq!(screen.load(PASSWORD)?.public_id(), id);
    assert_eq!(input.load(PASSWORD)?.public_id(), id);
    assert!(input.load(b"wrong test password").is_err());
    assert!(SessionVault::lock(&path, SessionKind::Screen).is_err());
    assert!(SessionVault::lock(&path, SessionKind::Input).is_err());
    probe(&path, "screen", false)?;
    probe(&path, "input", false)?;
    probe(&path, "exclusive", false)?;
    assert!(Vault::lock(&path).is_err());
    drop(screen);
    probe(&path, "screen", true)?;
    probe(&path, "input", false)?;
    probe(&path, "exclusive", false)?;
    drop(input);
    probe(&path, "exclusive", true)?;
    let exclusive = Vault::lock(&path)?;
    assert_eq!(std::fs::read(&path)?, original);
    exclusive.rename_device(PASSWORD, "Renamed fixture")?;
    assert_eq!(
        exclusive.load(PASSWORD)?.device_name(),
        Some("Renamed fixture")
    );
    Ok(())
}

#[test]
fn invalid_capability_path_does_not_block_other_lane_or_mutation() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("profile.vault");
    let profile = Profile::new(Role::Controller);
    Vault::lock(&path)?.initialize(&profile, PASSWORD)?;
    let invalid = path.with_file_name("profile.vault.input-session.lock");
    std::fs::create_dir(&invalid)?;
    assert!(SessionVault::lock(&path, SessionKind::Input).is_err());
    let screen = SessionVault::lock(&path, SessionKind::Screen)?;
    drop(screen);
    let exclusive = Vault::lock(&path)
        .context("Failed capability admission must release shared profile lease")?;
    assert_eq!(exclusive.load(PASSWORD)?.public_id(), profile.public_id());
    Ok(())
}
