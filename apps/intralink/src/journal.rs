//! Optional local journal for existing foreground operations. Never uploaded.
use std::{
    fs::{self, File, OpenOptions},
    future::Future,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use intralink_core::journal::{
    self, Attempt, MAX_HISTORY_BYTES, MAX_LINE_BYTES, Operation, Outcome,
};

fn history_path(vault: &Path) -> Result<PathBuf> {
    let mut name = vault
        .file_name()
        .context("Vault requires a file name")?
        .to_os_string();
    name.push(".history");
    Ok(vault.with_file_name(name))
}

fn inspect(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "History must be a regular file, not a symbolic link"
            );
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("Could not inspect local history"),
    }
}

fn open(path: &Path, create: bool) -> Result<File> {
    let exists = inspect(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    if create {
        options.append(true);
        // Create exclusively if missing; do not follow a path created between
        // the metadata check and the first creation attempt.
        options.create_new(!exists);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .context("Could not open local session history")?;
    ensure!(file.metadata()?.is_file(), "History must be a regular file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata()?;
        ensure!(
            meta.mode() & 0o077 == 0 && meta.nlink() == 1,
            "History must have private permissions and exactly one hard link"
        );
    }
    // A writer holds the exclusive lock for the whole attempt. Readers
    // take a shared lock, so neither sees an in-progress append.
    if create {
        fs2::FileExt::try_lock_exclusive(&file)
            .context("History is busy; close the other local command")?;
    } else {
        fs2::FileExt::try_lock_shared(&file)
            .context("History is busy; close the other local command")?;
    }
    Ok(file)
}

fn read(file: &mut File) -> Result<(usize, Vec<Attempt>)> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take((MAX_HISTORY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let attempts = journal::parse(&bytes)?;
    Ok((bytes.len(), attempts))
}

struct Recording {
    file: File,
    attempt: Attempt,
    start: Instant,
}

impl Recording {
    fn begin(vault: &Path, operation: Operation) -> Result<Self> {
        let path = history_path(vault)?;
        // As for the vault, use only a directory controlled by this OS user.
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let mut file = open(&path, true)?;
        let (used, attempts) = read(&mut file)?;
        // Reserve both records up front; never silently delete old entries.
        ensure!(
            used <= MAX_HISTORY_BYTES - 2 * MAX_LINE_BYTES,
            "History is full; archive it locally before recording another attempt"
        );
        let id = attempts
            .last()
            .map_or(Some(1), |a| a.id.checked_add(1))
            .context("History identifier exhausted")?;
        let started_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System clock precedes Unix epoch")?
            .as_secs();
        let attempt = Attempt {
            id,
            started_unix,
            operation,
            end: None,
        };
        file.write_all(attempt.start_line().as_bytes())?;
        file.sync_all()?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(Self {
            file,
            attempt,
            start: Instant::now(),
        })
    }

    fn finish(mut self, outcome: Outcome) -> Result<()> {
        let elapsed =
            u64::try_from(self.start.elapsed().as_millis()).context("History duration overflow")?;
        self.file
            .write_all(self.attempt.end_line(outcome, elapsed).as_bytes())?;
        self.file.sync_all()?;
        Ok(())
    }
}

/// A recording is an attempt, not proof of authentication or of transmitted
/// pixels. The command is not polled if the requested journal cannot start.
pub async fn run(
    vault: &Path,
    enabled: bool,
    operation: Operation,
    future: impl Future<Output = Result<Outcome>>,
) -> Result<()> {
    if !enabled {
        return future.await.map(|_| ());
    }
    let recording = Recording::begin(vault, operation)?;
    let result = future.await;
    let outcome = match &result {
        Ok(outcome) => *outcome,
        Err(_) => Outcome::Failed,
    };
    let written = recording.finish(outcome);
    match (result, written) {
        (Ok(_), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => {
            Err(error
                .context("Command returned, but local history completion could not be recorded"))
        }
        (Err(error), Err(history)) => {
            eprintln!("Could not record history completion: {history:#}");
            Err(error)
        }
    }
}

pub fn show(vault: &Path) -> Result<()> {
    let path = history_path(vault)?;
    if !inspect(&path)? {
        println!(
            "No local session history. Enable it for a foreground command with --record-session."
        );
        return Ok(());
    }
    let (_, attempts) = read(&mut open(&path, false)?)?;
    println!("Local history (UTC Unix seconds; attempt records, not proof of access):");
    for a in &attempts {
        match a.end {
            Some((outcome, elapsed)) => println!(
                "{}\t{}\t{}\t{}\t{} ms",
                a.id,
                a.started_unix,
                a.operation.label(),
                outcome.label(),
                elapsed
            ),
            None => println!(
                "{}\t{}\t{}\tunknown outcome",
                a.id,
                a.started_unix,
                a.operation.label()
            ),
        }
    }
    println!(
        "{} attempt(s). No peer IDs, addresses or payloads are stored.",
        attempts.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_persists_and_interruption_is_unknown() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let vault = dir.path().join("test.vault");
        Recording::begin(&vault, Operation::ViewHost)?.finish(Outcome::Cancelled)?;
        drop(Recording::begin(&vault, Operation::ProbeHost)?);
        Recording::begin(&vault, Operation::ProbeController)?.finish(Outcome::Failed)?;
        let (_, events) = read(&mut open(&history_path(&vault)?, false)?)?;
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].end.map(|e| e.0), Some(Outcome::Cancelled));
        assert_eq!(events[1].end, None);
        assert_eq!(events[2].end.map(|e| e.0), Some(Outcome::Failed));
        assert!(!vault.exists());
        Ok(())
    }

    #[tokio::test]
    async fn opt_out_does_not_create_file_and_corruption_blocks_operation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let vault = dir.path().join("test.vault");
        let path = history_path(&vault)?;
        run(&vault, false, Operation::ProbeHost, async {
            Ok(Outcome::Completed)
        })
        .await?;
        assert!(!path.exists());
        let interrupted = Recording::begin(&vault, Operation::ProbeHost)?;
        drop(interrupted);
        let mut file = OpenOptions::new().append(true).open(&path)?;
        file.write_all(b"partial")?;
        drop(file);
        let mut polled = false;
        let result = run(&vault, true, Operation::ProbeHost, async {
            polled = true;
            Ok(Outcome::Completed)
        })
        .await;
        assert!(result.is_err());
        assert!(!polled);
        Ok(())
    }

    #[tokio::test]
    async fn wrapper_records_completion_and_preserves_command_error() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let vault = dir.path().join("test.vault");
        run(&vault, true, Operation::ProbeHost, async {
            Ok(Outcome::Completed)
        })
        .await?;
        let result = run(&vault, true, Operation::ViewController, async {
            anyhow::bail!("expected command error")
        })
        .await;
        assert_eq!(result.unwrap_err().to_string(), "expected command error");
        let (_, events) = read(&mut open(&history_path(&vault)?, false)?)?;
        assert_eq!(events[0].end.map(|e| e.0), Some(Outcome::Completed));
        assert_eq!(events[1].end.map(|e| e.0), Some(Outcome::Failed));
        Ok(())
    }

    #[test]
    fn full_or_busy_journal_is_not_overwritten() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let vault = dir.path().join("test.vault");
        let path = history_path(&vault)?;
        let active = Recording::begin(&vault, Operation::ProbeHost)?;
        assert!(Recording::begin(&vault, Operation::ProbeHost).is_err());
        assert!(open(&path, false).is_err());
        drop(active);
        let mut content = String::new();
        let mut id = 1;
        while content.len() <= MAX_HISTORY_BYTES - 2 * MAX_LINE_BYTES {
            content.push_str(
                &Attempt {
                    id,
                    started_unix: 1,
                    operation: Operation::ViewHost,
                    end: None,
                }
                .start_line(),
            );
            id += 1;
        }
        fs::write(&path, content.as_bytes())?;
        assert!(Recording::begin(&vault, Operation::ProbeHost).is_err());
        assert_eq!(fs::read(&path)?, content.as_bytes());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_and_shared_file_are_refused() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir()?;
        let vault = dir.path().join("test.vault");
        let path = history_path(&vault)?;
        let target = dir.path().join("unrelated");
        fs::write(&target, b"keep")?;
        symlink(&target, &path)?;
        assert!(Recording::begin(&vault, Operation::ProbeHost).is_err());
        assert_eq!(fs::read(&target)?, b"keep");
        fs::remove_file(&path)?;
        fs::write(&path, b"")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
        assert!(Recording::begin(&vault, Operation::ProbeHost).is_err());
        Ok(())
    }
}
