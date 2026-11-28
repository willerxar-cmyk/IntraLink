//! Local presentation preference. Never supplies routes, grants, or an active target.
//! The workspace must hold the exclusive input lane and shared profile lease.
use super::portal::Mode;
use anyhow::{Context, Result, ensure};
use intralink_core::{input::MAX_SLOTS, input_layout::Layout};
use iroh::EndpointId;
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAGIC: &str = "ILKVM1";
const MAX_BYTES: u64 = 2048;

pub(super) struct Store {
    path: PathBuf,
    base: EndpointId,
    peers: Vec<EndpointId>,
}

#[derive(Clone, Copy)]
pub(super) struct Preference {
    pub layout: Layout,
    pub portal: Mode,
}

impl Store {
    pub fn new(vault: &Path, base: EndpointId, peers: Vec<EndpointId>) -> Result<Self> {
        ensure!(
            (1..=MAX_SLOTS).contains(&peers.len()),
            "Invalid layout peer count"
        );
        let unique: BTreeSet<_> = peers.iter().collect();
        ensure!(
            unique.len() == peers.len() && !unique.contains(&base),
            "Invalid layout identities"
        );
        let parent = vault
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = parent
            .canonicalize()
            .context("Resolve local layout directory")?;
        let mut filename = vault
            .file_name()
            .context("Missing vault filename")?
            .to_os_string();
        filename.push(".kvm-layout");
        Ok(Self {
            path: parent.join(filename),
            base,
            peers,
        })
    }

    pub fn defaults(&self) -> Preference {
        Preference {
            layout: Layout::arranged(self.peers.len()).expect("validated slot count"),
            portal: Mode::default(),
        }
    }

    fn regular_file(&self) -> Result<bool> {
        match fs::symlink_metadata(&self.path) {
            Ok(meta) => {
                ensure!(meta.file_type().is_file(), "Layout must be a regular file");
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).context("Inspect local layout"),
        }
    }

    pub fn load(&self) -> Result<Option<Preference>> {
        if !self.regular_file()? {
            return Ok(None);
        }
        let file = File::open(&self.path).context("Open local layout")?;
        ensure!(file.metadata()?.is_file(), "Layout must be a regular file");
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 <= MAX_BYTES, "Layout exceeds size limit");
        self.decode(std::str::from_utf8(&bytes)?)
    }

    fn decode(&self, value: &str) -> Result<Option<Preference>> {
        ensure!(
            value.ends_with('\n') && !value.contains('\r'),
            "Noncanonical layout"
        );
        let mut lines = value.lines();
        ensure!(lines.next() == Some(MAGIC), "Unsupported layout version");
        let base = parse_id(lines.next().context("Missing BASE identity")?)?;
        let portal = match lines.next() {
            Some("FULL") => Mode::Full,
            Some("REDUCED") => Mode::Reduced,
            Some("OFF") => Mode::Off,
            _ => anyhow::bail!("Unknown portal preference"),
        };
        let mut ids = [None; 9];
        let mut unique = BTreeSet::new();
        for cell in &mut ids {
            let line = lines.next().context("Incomplete layout")?;
            if line != "-" {
                let id = parse_id(line)?;
                ensure!(unique.insert(id), "Duplicate layout identity");
                *cell = Some(id);
            }
        }
        ensure!(lines.next().is_none(), "Unexpected layout data");
        ensure!(
            unique.contains(&base) && (2..=9).contains(&unique.len()),
            "Missing BASE or CLIENT"
        );
        let expected: BTreeSet<_> = self.peers.iter().copied().chain([self.base]).collect();
        // A different controller or peer set never selects a stale slot.
        if base != self.base || unique != expected {
            return Ok(None);
        }
        let cells = ids.map(|id| {
            id.map(|id| {
                if id == self.base {
                    0
                } else {
                    self.peers
                        .iter()
                        .position(|peer| *peer == id)
                        .expect("matched peer set")
                        + 1
                }
            })
        });
        let layout =
            Layout::from_arrangement(self.peers.len(), cells).context("Invalid arrangement")?;
        Ok(Some(Preference { layout, portal }))
    }

    pub fn save(&self, preference: Preference) -> Result<()> {
        ensure!(
            preference.layout.cells() == 9,
            "Only spatial arrangements can be stored"
        );
        let mut value = format!("{MAGIC}\n{}\n{}\n", self.base, preference.portal.label());
        for cell in 0..9 {
            let id = match preference.layout.label(cell) {
                None => None,
                Some(0) => Some(self.base),
                Some(label) => Some(*self.peers.get(label - 1).context("Unknown layout slot")?),
            };
            value.push_str(&id.map_or_else(|| "-".to_owned(), |id| id.to_string()));
            value.push('\n');
        }
        ensure!(
            self.decode(&value)?.is_some(),
            "Layout identity set mismatch"
        );
        self.regular_file()?;
        let mut temp = tempfile::NamedTempFile::new_in(
            self.path.parent().context("Missing layout directory")?,
        )?;
        temp.write_all(value.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path)
            .map_err(|_| anyhow::anyhow!("Could not replace local layout"))?;
        Ok(())
    }
}

fn parse_id(value: &str) -> Result<EndpointId> {
    let id: EndpointId = value.parse().context("Invalid layout identity")?;
    ensure!(id.to_string() == value, "Noncanonical layout identity");
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn id(value: u8) -> EndpointId {
        SecretKey::from_bytes(&[value; 32]).public()
    }

    #[test]
    fn preferences_survive_reopen_and_peer_order_changes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.vault");
        let store = Store::new(&path, id(1), vec![id(2), id(3)])?;
        assert!(store.load()?.is_none());
        let mut prefs = store.defaults();
        assert!(prefs.layout.relocate(3, 0));
        assert!(prefs.layout.relocate(4, 7));
        prefs.portal = Mode::Reduced;
        store.save(prefs)?;
        let reopened = Store::new(&path, id(1), vec![id(3), id(2)])?
            .load()?
            .unwrap();
        assert_eq!(reopened.layout.cell_of(None), Some(7));
        assert_eq!(reopened.layout.cell_of(Some(1)), Some(0));
        assert_eq!(reopened.layout.cell_of(Some(0)), Some(5));
        assert!(reopened.portal == Mode::Reduced);
        let bytes = fs::read(&store.path)?;
        assert!(
            Store::new(&path, id(4), vec![id(2), id(3)])?
                .load()?
                .is_none()
        );
        assert!(Store::new(&path, id(1), vec![id(2)])?.load()?.is_none());
        assert!(
            Store::new(&path, id(1), vec![id(2), id(5)])?
                .load()?
                .is_none()
        );
        assert_eq!(fs::read(&store.path)?, bytes);
        prefs.portal = Mode::Off;
        store.save(prefs)?;
        assert!(store.load()?.unwrap().portal == Mode::Off);
        Ok(())
    }

    #[test]
    fn rejects_corrupt_duplicate_truncated_and_oversized_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::new(&dir.path().join("test.vault"), id(1), vec![id(2)])?;
        store.save(store.defaults())?;
        let valid = fs::read_to_string(&store.path)?;
        for bad in [
            valid.replace(MAGIC, "ILKVM2"),
            valid.replace("FULL", "UNKNOWN"),
            valid.replace(&id(2).to_string(), &id(1).to_string()),
            valid.trim_end().to_string(),
            format!("{valid}extra\n"),
            "x".repeat(MAX_BYTES as usize + 1),
        ] {
            fs::write(&store.path, bad.as_bytes())?;
            assert!(store.load().is_err());
            assert_eq!(fs::read(&store.path)?, bad.as_bytes());
        }
        fs::remove_file(&store.path)?;
        fs::create_dir(&store.path)?;
        assert!(store.load().is_err());
        assert!(store.save(store.defaults()).is_err());
        assert!(store.path.is_dir());
        assert!(Store::new(&dir.path().join("test.vault"), id(1), vec![id(1)]).is_err());
        assert!(Store::new(&dir.path().join("test.vault"), id(1), vec![id(2), id(2)]).is_err());
        Ok(())
    }
}
