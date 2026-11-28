//! Shared read-only capability leases; exclusive authenticated ciphertext writes.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use tempfile::NamedTempFile;

use crate::{Profile, RoleChange, envelope};

pub struct Vault {
    path: PathBuf,
    // Lock released automatically when the file handle is dropped.
    _lock: File,
}

/// A read-only profile lease for one explicitly approved desktop capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Screen,
    Input,
}

impl SessionKind {
    fn suffix(self) -> &'static str {
        match self {
            Self::Screen => ".screen-session.lock",
            Self::Input => ".input-session.lock",
        }
    }
}

/// Holds an exclusive capability lock and a shared profile lock for its
/// entire lifetime. There is deliberately no initialize/save/rename/revoke
/// method on this type. Existing exclusive Vault users remain incompatible
/// with any running session, including older binaries and Android.
pub struct SessionVault {
    path: PathBuf,
    // Drop the capability lease before releasing the shared profile lease.
    _capability: File,
    _profile: File,
}

struct LockPaths {
    path: PathBuf,
    profile: PathBuf,
}

impl LockPaths {
    fn resolve(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).context("Could not create vault directory")?;
        let parent = parent
            .canonicalize()
            .context("Could not resolve vault directory")?;
        let filename = path.file_name().context("Vault requires a file name")?;
        let path = parent.join(filename);
        reject_symlink(&path)?;
        let profile = sidecar(&path, ".lock")?;
        Ok(Self { path, profile })
    }
}

fn sidecar(path: &Path, suffix: &str) -> Result<PathBuf> {
    let mut filename = path
        .file_name()
        .context("Vault requires a file name")?
        .to_os_string();
    filename.push(suffix);
    Ok(path.with_file_name(filename))
}

fn open_lock(path: &Path) -> Result<File> {
    reject_symlink(path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(path)
        .context("Could not open local vault lock")?;
    ensure!(
        lock.metadata()?.is_file(),
        "Vault lock must be a regular file"
    );
    Ok(lock)
}

fn load_profile(path: &Path, password: &[u8]) -> Result<Profile> {
    reject_symlink(path)?;
    let file = File::open(path).context("Could not open vault; initialize it first")?;
    ensure!(file.metadata()?.is_file(), "Vault must be a regular file");
    let mut bytes = Vec::new();
    file.take(envelope::MAX_FILE_SIZE as u64 + 1)
        .read_to_end(&mut bytes)?;
    envelope::open(&bytes, password)
}

impl SessionVault {
    pub fn lock(path: &Path, kind: SessionKind) -> Result<Self> {
        let paths = LockPaths::resolve(path)?;
        // All callers acquire the original profile lock first. A mutating
        // command can never interleave between capability admission and read.
        let profile = open_lock(&paths.profile)?;
        fs2::FileExt::try_lock_shared(&profile)
            .context("Vault is being edited or used by an exclusive command")?;
        // Do not create a session lane for an uninitialized profile.
        ensure!(
            paths.path.is_file(),
            "Could not open vault; initialize it first"
        );
        let capability = open_lock(&sidecar(&paths.path, kind.suffix())?)?;
        fs2::FileExt::try_lock_exclusive(&capability).with_context(|| {
            format!("{kind:?} capability already in use; stop the other local command")
        })?;
        Ok(Self {
            path: paths.path,
            _capability: capability,
            _profile: profile,
        })
    }

    pub fn load(&self, password: &[u8]) -> Result<Profile> {
        load_profile(&self.path, password)
    }
}

impl Vault {
    pub fn lock(path: &Path) -> Result<Self> {
        let paths = LockPaths::resolve(path)?;
        let lock = open_lock(&paths.profile)?;
        fs2::FileExt::try_lock_exclusive(&lock).context(
            "Vault already in use; close active screen/input sessions or the other local command",
        )?;
        Ok(Self {
            path: paths.path,
            _lock: lock,
        })
    }

    pub fn initialize(&self, profile: &Profile, password: &[u8]) -> Result<()> {
        self.write(profile, password, false)
    }

    pub fn load(&self, password: &[u8]) -> Result<Profile> {
        load_profile(&self.path, password)
    }

    /// Authenticate the current ciphertext before replacing it. A normal save
    /// must preserve the installation identity and role; key rotation or a
    /// passphrase change requires a separate, explicitly authorized operation.
    pub fn save(&self, profile: &Profile, password: &[u8]) -> Result<()> {
        let current = self.load(password)?;
        ensure!(
            current.public_id() == profile.public_id(),
            "Cannot replace vault identity"
        );
        ensure!(
            current.role() == profile.role(),
            "Cannot replace vault role"
        );
        drop(current);
        self.write(profile, password, true)
    }

    /// Explicit authenticated role transition, independent of ordinary save.
    /// The caller reviews the expected impact while holding the same vault lock.
    pub fn change_role(&self, password: &[u8], expected: RoleChange) -> Result<RoleChange> {
        let mut current = self.load(password)?;
        let impact = current.role_change_impact(expected.next);
        ensure!(
            impact == expected,
            "Role change impact changed; review it again"
        );
        if impact.previous != impact.next {
            current.apply_role_change(impact.next);
            self.write(&current, password, true)?;
        }
        Ok(impact)
    }

    /// Rename without accepting a replacement identity or role from the caller.
    pub fn rename_device(&self, password: &[u8], name: &str) -> Result<()> {
        let mut current = self.load(password)?;
        current.rename_device(name)?;
        self.write(&current, password, true)
    }

    fn write(&self, profile: &Profile, password: &[u8], replace: bool) -> Result<()> {
        reject_symlink(&self.path)?;
        let bytes = envelope::seal(profile, password)?;
        let parent = self.path.parent().context("Missing vault directory")?;
        let mut staging = NamedTempFile::new_in(parent).context("Could not stage vault update")?;
        staging.write_all(&bytes)?;
        staging.as_file().sync_all()?;
        if replace {
            staging
                .persist(&self.path)
                .map_err(|_| anyhow::anyhow!("Could not replace vault"))?;
        } else {
            staging.persist_noclobber(&self.path).map_err(|_| {
                anyhow::anyhow!("Could not create vault; existing vault is never overwritten")
            })?;
        }
        #[cfg(unix)]
        File::open(parent)?
            .sync_all()
            .context("Vault was written, but directory sync failed")?;
        Ok(())
    }
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            !metadata.file_type().is_symlink() && metadata.is_file(),
            "Vault and lock paths must be regular files"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Could not inspect local vault path"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Direction, Peer, Route};
    use intralink_core::Role;

    #[test]
    fn pending_offer_and_replay_state_survive_vault_reopen() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let password = b"local test passphrase";
        let host_path = directory.path().join("host.vault");
        let controller_path = directory.path().join("controller.vault");
        let mut host = Profile::new(Role::Host);
        let controller = Profile::new(Role::Controller);
        let offer = host.create_invite(
            controller.public_id(),
            Route::new("localhost", 45100)?,
            1000,
            300,
        )?;
        Vault::lock(&host_path)?.initialize(&host, password)?;
        Vault::lock(&controller_path)?.initialize(&controller, password)?;
        drop(host);
        drop(controller);
        let acceptance = {
            let vault = Vault::lock(&controller_path)?;
            let mut controller = vault.load(password)?;
            let acceptance = controller.accept_invite("host", &offer, 1010)?;
            controller.revoke("host")?;
            vault.save(&controller, password)?;
            acceptance
        };
        {
            let vault = Vault::lock(&host_path)?;
            let mut host = vault.load(password)?;
            host.confirm_invite("controller", &acceptance, 1020)?;
            host.revoke("controller")?;
            vault.save(&host, password)?;
        }
        assert!(
            Vault::lock(&host_path)?
                .load(password)?
                .confirm_invite("controller", &acceptance, 1030)
                .is_err()
        );
        assert!(
            Vault::lock(&controller_path)?
                .load(password)?
                .accept_invite("host", &offer, 1030)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn persistent_identity_and_revocation() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("profile.vault");
        let password = b"local test passphrase";
        let owner = Profile::new(Role::Controller);
        let host_id;
        {
            let vault = Vault::lock(&path)?;
            let mut profile = Profile::new(Role::Host);
            host_id = profile.public_id();
            profile.add_peer(Peer::new(
                "owner",
                owner.public_id(),
                Direction::Inbound,
                None,
            )?)?;
            vault.initialize(&profile, password)?;
            assert!(
                vault
                    .initialize(&Profile::new(Role::Host), password)
                    .is_err()
            );
            assert!(Vault::lock(&path).is_err());
        }
        {
            let vault = Vault::lock(&path)?;
            let mut profile = vault.load(password)?;
            assert_eq!(profile.public_id(), host_id);
            assert_eq!(
                profile.peer("owner", Direction::Inbound)?.identity()?,
                owner.public_id()
            );
            profile.revoke("owner")?;
            vault.save(&profile, password)?;
        }
        let vault = Vault::lock(&path)?;
        let profile = vault.load(password)?;
        assert_eq!(profile.public_id(), host_id);
        assert!(profile.peer("owner", Direction::Inbound).is_err());
        Ok(())
    }

    #[test]
    fn authenticated_role_changes_preserve_identity_and_replay_state() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("profile.vault");
        let password = b"local test passphrase";
        let vault = Vault::lock(&path)?;
        let mut local = Profile::new_named(Role::Hybrid, "Workstation")?;
        let mut remote = Profile::new(Role::Hybrid);
        let another = Profile::new(Role::Controller);
        let route = Route::new("localhost", 45100)?;
        let offer = remote.create_invite(local.public_id(), route.clone(), 1000, 300)?;
        local.accept_invite("remote", &offer, 1010)?;
        local.add_peer(Peer::new(
            "owner",
            another.public_id(),
            Direction::Inbound,
            None,
        )?)?;
        let pending = local.create_invite(remote.public_id(), route, 1011, 300)?;
        let acceptance = remote.accept_invite("local", &pending, 1012)?;
        let identity = local.public_id();
        vault.initialize(&local, password)?;
        let impact = local.role_change_impact(Role::Host);
        assert_eq!(impact.revoked_grants, 1);
        assert!(impact.canceled_invite);
        let original = fs::read(&path)?;
        assert!(vault.change_role(b"wrong test passphrase", impact).is_err());
        assert_eq!(fs::read(&path)?, original);
        let stale = RoleChange {
            revoked_grants: 0,
            ..impact
        };
        assert!(vault.change_role(password, stale).is_err());
        assert_eq!(fs::read(&path)?, original);
        assert_eq!(vault.change_role(password, impact)?, impact);
        let mut client = vault.load(password)?;
        assert_eq!(client.public_id(), identity);
        assert_eq!(client.device_name(), Some("Workstation"));
        assert_eq!(client.role(), Role::Host);
        assert!(client.peer("owner", Direction::Inbound).is_ok());
        assert!(client.peer("remote", Direction::Outbound).is_err());
        assert!(client.confirm_invite("late", &acceptance, 1013).is_err());
        let next = client.role_change_impact(Role::Controller);
        assert_eq!(next.revoked_grants, 1);
        vault.change_role(password, next)?;
        let mut base = vault.load(password)?;
        assert_eq!(base.public_id(), identity);
        assert!(base.peers().is_empty());
        assert!(base.accept_invite("replay", &offer, 1014).is_err());
        let unchanged = fs::read(&path)?;
        vault.change_role(password, base.role_change_impact(Role::Controller))?;
        assert_eq!(fs::read(&path)?, unchanged);
        vault.rename_device(password, "Desk 2")?;
        assert_eq!(vault.load(password)?.device_name(), Some("Desk 2"));
        assert_eq!(vault.load(password)?.public_id(), identity);
        let renamed = fs::read(&path)?;
        assert!(vault.rename_device(password, "bad\nname").is_err());
        assert_eq!(fs::read(&path)?, renamed);
        Ok(())
    }

    #[test]
    fn normal_save_still_rejects_role_changes() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("profile.vault");
        let password = b"local test passphrase";
        let vault = Vault::lock(&path)?;
        let mut profile = Profile::new(Role::Host);
        vault.initialize(&profile, password)?;
        let original = fs::read(&path)?;
        profile.apply_role_change(Role::Hybrid);
        assert!(vault.save(&profile, password).is_err());
        assert_eq!(fs::read(&path)?, original);
        Ok(())
    }

    #[test]
    fn save_rejects_wrong_password_and_identity_without_overwriting() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("profile.vault");
        let password = b"local test passphrase";
        let vault = Vault::lock(&path)?;
        let profile = Profile::new(Role::Host);
        vault.initialize(&profile, password)?;
        let original = fs::read(&path)?;
        assert!(vault.save(&profile, b"different test passphrase").is_err());
        assert_eq!(fs::read(&path)?, original);
        assert!(vault.save(&Profile::new(Role::Host), password).is_err());
        assert_eq!(fs::read(&path)?, original);
        assert_eq!(vault.load(password)?.public_id(), profile.public_id());
        Ok(())
    }

    #[test]
    fn save_rejects_corrupted_existing_ciphertext() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("profile.vault");
        let password = b"local test passphrase";
        let vault = Vault::lock(&path)?;
        let profile = Profile::new(Role::Controller);
        vault.initialize(&profile, password)?;
        let mut damaged = fs::read(&path)?;
        *damaged.last_mut().context("Missing authentication tag")? ^= 1;
        fs::write(&path, &damaged)?;
        assert!(vault.save(&profile, password).is_err());
        assert_eq!(fs::read(&path)?, damaged);
        Ok(())
    }
}
