//! Identity-bound, local-only geometry preference. The exclusive input lane
//! and shared profile lease are held by the caller; no routes or grants.
use super::portal::Mode;
use anyhow::{Context, Result, ensure};
use intralink_core::{
    display_geometry::{self, Display, Placement, Viewport},
    input::MAX_SLOTS,
};
use iroh::EndpointId;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAGIC: &str = "ILGEOM1";
const MAX_BYTES: u64 = 3072;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Observed {
    pub monitor: u32,
    pub display: Display,
}

/// Read only metadata for the single primary monitor, never capture pixels.
/// Dimensions and scale are the OS/backend's reported values.
pub(crate) fn observe_base() -> Result<Observed> {
    let monitors = xcap::Monitor::all().map_err(|e| anyhow::anyhow!("Monitor enumeration: {e}"))?;
    let mut selected = None;
    for monitor in monitors {
        if monitor
            .is_primary()
            .map_err(|e| anyhow::anyhow!("Primary monitor: {e}"))?
        {
            ensure!(selected.is_none(), "Ambiguous primary monitor");
            selected = Some(monitor);
        }
    }
    let m = selected.context("No primary monitor available")?;
    let width = m
        .width()
        .map_err(|e| anyhow::anyhow!("Monitor width: {e}"))?
        .try_into()?;
    let height = m
        .height()
        .map_err(|e| anyhow::anyhow!("Monitor height: {e}"))?
        .try_into()?;
    let scale = m
        .scale_factor()
        .map_err(|e| anyhow::anyhow!("Monitor scale: {e}"))?
        * 100.0;
    let rotation = m
        .rotation()
        .map_err(|e| anyhow::anyhow!("Monitor rotation: {e}"))?;
    ensure!(
        scale.is_finite()
            && (50.0..=400.0).contains(&scale)
            && matches!(rotation, 0.0 | 90.0 | 180.0 | 270.0),
        "Unsupported primary monitor scale or orientation"
    );
    let display = Display {
        width,
        height,
        scale: scale.round() as u16,
        rotation: rotation as u16,
    };
    ensure!(
        display.logical().is_some(),
        "Unsupported primary monitor dimensions"
    );
    Ok(Observed {
        monitor: m.id().map_err(|e| anyhow::anyhow!("Monitor ID: {e}"))?,
        display,
    })
}

#[derive(Clone)]
pub(super) struct Config {
    pub placements: Vec<Placement>,
    pub portal: Mode,
}

pub(super) struct Store {
    path: PathBuf,
    identities: Vec<EndpointId>, // BASE first; slot order is not persistence authority
    observed: Observed,
    displays: Option<Vec<Display>>,
}

impl Store {
    pub fn new(
        vault: &Path,
        base: EndpointId,
        peers: Vec<EndpointId>,
        observed: Observed,
    ) -> Result<Self> {
        ensure!(
            (1..=MAX_SLOTS).contains(&peers.len()),
            "Select 1-8 distinct peers"
        );
        ensure!(
            observed.display.logical().is_some(),
            "Invalid observed BASE"
        );
        let identities: Vec<_> = std::iter::once(base).chain(peers).collect();
        ensure!(
            identities.iter().collect::<BTreeSet<_>>().len() == identities.len(),
            "Duplicate topology identity"
        );
        let parent = vault
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()?;
        let mut filename = vault
            .file_name()
            .context("Missing vault filename")?
            .to_os_string();
        filename.push(".kvm-topology");
        Ok(Self {
            path: parent.join(filename),
            identities,
            observed,
            displays: None,
        })
    }

    /// Only the current authenticated input/2 session can supply this list.
    /// It is never restored as trusted from the preference file.
    pub fn with_displays(mut self, displays: Vec<Display>) -> Result<Self> {
        ensure!(
            displays.len() + 1 == self.slots(),
            "Missing authenticated CLIENT geometry"
        );
        ensure!(
            displays.iter().all(|display| display.logical().is_some()),
            "Invalid authenticated CLIENT geometry"
        );
        self.displays = Some(displays);
        self.validate(&self.defaults())?;
        Ok(self)
    }

    pub fn authenticated(&self) -> bool {
        self.displays.is_some()
    }

    pub fn defaults(&self) -> Config {
        let base = self.observed.display;
        let mut placements = vec![Placement {
            id: 0,
            x: 0,
            y: 0,
            display: base,
        }];
        let mut right = i32::from(base.logical().expect("validated BASE").0);
        let mut left = 0;
        for index in 1..self.slots() {
            let display = self.displays.as_ref().map_or(
                Display {
                    width: 1280,
                    height: 720,
                    scale: 100,
                    rotation: 0,
                },
                |values| values[index - 1],
            );
            let width = i32::from(display.logical().expect("validated display").0);
            // Alternating sides keeps eight maximum-size screens inside
            // the signed logical coordinate bounds.
            let x = if self.displays.is_none() || index % 2 == 1 {
                let x = right;
                right += width;
                x
            } else {
                left -= width;
                left
            };
            placements.push(Placement {
                id: index as u8,
                x,
                y: 0,
                display,
            });
        }
        Config {
            placements,
            portal: Mode::Full,
        }
    }

    fn matches_displays(&self, config: &Config) -> bool {
        self.displays.as_ref().is_none_or(|displays| {
            config
                .placements
                .iter()
                .skip(1)
                .map(|p| p.display)
                .eq(displays.iter().copied())
        })
    }

    pub fn observed(&self) -> Observed {
        self.observed
    }

    pub fn slots(&self) -> usize {
        self.identities.len()
    }

    fn regular(&self) -> Result<bool> {
        match fs::symlink_metadata(&self.path) {
            Ok(meta) => {
                ensure!(
                    meta.file_type().is_file(),
                    "Topology must be a regular file"
                );
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub fn load(&self) -> Result<Option<Config>> {
        if !self.regular()? {
            return Ok(None);
        }
        let file = File::open(&self.path)?;
        ensure!(
            file.metadata()?.is_file(),
            "Topology must be a regular file"
        );
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "Topology exceeds size limit"
        );
        self.decode(std::str::from_utf8(&bytes)?)
    }

    fn decode(&self, value: &str) -> Result<Option<Config>> {
        ensure!(
            value.ends_with('\n') && !value.contains('\r'),
            "Noncanonical topology"
        );
        let mut lines = value.lines();
        ensure!(lines.next() == Some(MAGIC), "Unknown topology version");
        let base = parse_id(lines.next().context("Missing BASE")?)?;
        let monitor_text = lines.next().context("Missing monitor")?;
        let monitor: u32 = monitor_text.parse()?;
        ensure!(
            monitor.to_string() == monitor_text,
            "Noncanonical monitor ID"
        );
        let portal = match lines.next() {
            Some("FULL") => Mode::Full,
            Some("REDUCED") => Mode::Reduced,
            Some("OFF") => Mode::Off,
            _ => anyhow::bail!("Invalid portal mode"),
        };
        let mut rows = BTreeMap::new();
        for line in lines {
            ensure!(rows.len() < MAX_SLOTS + 1, "Too many topology rows");
            let fields: Vec<_> = line.split(' ').collect();
            ensure!(fields.len() == 7, "Invalid topology row");
            let identity = parse_id(fields[0])?;
            let p = Placement {
                id: 0,
                x: fields[1].parse()?,
                y: fields[2].parse()?,
                display: Display {
                    width: fields[3].parse()?,
                    height: fields[4].parse()?,
                    scale: fields[5].parse()?,
                    rotation: fields[6].parse()?,
                },
            };
            ensure!(
                p.screen().is_some() && row(identity, p) == line,
                "Noncanonical geometry"
            );
            ensure!(rows.insert(identity, p).is_none(), "Duplicate topology row");
        }
        ensure!(
            (2..=MAX_SLOTS + 1).contains(&rows.len()) && rows.contains_key(&base),
            "Missing topology BASE/CLIENT"
        );
        let base_placement = rows[&base];
        ensure!(
            base_placement.x == 0 && base_placement.y == 0,
            "BASE must anchor the topology"
        );
        // Validate even a stale file before deciding it is for another set.
        let mut original = vec![base_placement];
        for (id, p) in &rows {
            if *id != base {
                original.push(*p);
            }
        }
        for (i, p) in original.iter_mut().enumerate() {
            p.id = i as u8;
        }
        let old = display_geometry::topology(&original).context("Invalid saved topology")?;
        ensure!(old.connected(), "Saved topology has disconnected screens");
        if base != self.identities[0]
            || monitor != self.observed.monitor
            || base_placement.display != self.observed.display
            || rows.len() != self.identities.len()
            || self.identities.iter().any(|id| !rows.contains_key(id))
        {
            return Ok(None);
        }
        let placements = self
            .identities
            .iter()
            .enumerate()
            .map(|(i, id)| Placement {
                id: i as u8,
                ..rows[id]
            })
            .collect::<Vec<_>>();
        let config = Config { placements, portal };
        if !self.matches_displays(&config) {
            return Ok(None);
        }
        self.validate(&config)?;
        Ok(Some(config))
    }

    fn validate(&self, config: &Config) -> Result<()> {
        ensure!(
            config.placements.len() == self.identities.len(),
            "Topology peer set mismatch"
        );
        ensure!(
            config.placements[0].display == self.observed.display,
            "BASE monitor geometry changed"
        );
        ensure!(
            self.matches_displays(config),
            "Authenticated CLIENT geometry changed; review again"
        );
        let topology = display_geometry::topology(&config.placements)
            .context("Invalid or overlapping topology")?;
        ensure!(
            topology.connected(),
            "Every CLIENT must have a shared-edge path to BASE"
        );
        ensure!(
            Viewport::fit(&topology, 928, 430).is_some()
                && Viewport::fit(&topology, 600, 390).is_some(),
            "Topology too small to display safely"
        );
        Ok(())
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        self.validate(config)?;
        let mut value = format!(
            "{MAGIC}\n{}\n{}\n{}\n",
            self.identities[0],
            self.observed.monitor,
            config.portal.label()
        );
        for (id, p) in self
            .identities
            .iter()
            .zip(config.placements.iter().copied())
        {
            value.push_str(&row(*id, p));
            value.push('\n');
        }
        ensure!(
            value.len() as u64 <= MAX_BYTES && self.decode(&value)?.is_some(),
            "Invalid serialized topology"
        );
        self.regular()?;
        let mut temp = tempfile::NamedTempFile::new_in(
            self.path.parent().context("Missing topology directory")?,
        )?;
        temp.write_all(value.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path)
            .map_err(|_| anyhow::anyhow!("Could not replace topology preference"))?;
        Ok(())
    }
}

fn row(id: EndpointId, p: Placement) -> String {
    format!(
        "{id} {} {} {} {} {} {}",
        p.x, p.y, p.display.width, p.display.height, p.display.scale, p.display.rotation
    )
}
fn parse_id(value: &str) -> Result<EndpointId> {
    let id: EndpointId = value.parse().context("Invalid topology identity")?;
    ensure!(id.to_string() == value, "Noncanonical topology identity");
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;
    fn id(n: u8) -> EndpointId {
        SecretKey::from_bytes(&[n; 32]).public()
    }
    fn display(w: u16, h: u16, scale: u16, rotation: u16) -> Display {
        Display {
            width: w,
            height: h,
            scale,
            rotation,
        }
    }

    #[test]
    fn authenticated_dimensions_override_stale_local_geometry_only_after_explicit_save()
    -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.vault");
        let base = Observed {
            monitor: 1,
            display: display(1920, 1080, 100, 0),
        };
        let old = Store::new(&path, id(1), vec![id(2), id(3)], base)?;
        old.save(&old.defaults())?;
        let original = fs::read(&old.path)?;
        let displays = vec![display(1080, 1920, 125, 90), display(2560, 1440, 150, 0)];
        let store =
            Store::new(&path, id(1), vec![id(2), id(3)], base)?.with_displays(displays.clone())?;
        assert!(store.authenticated());
        assert!(store.load()?.is_none());
        assert_eq!(fs::read(&old.path)?, original);
        assert!(store.save(&old.defaults()).is_err());
        let config = store.defaults();
        assert_eq!(config.placements[1].display, displays[0]);
        assert_eq!(config.placements[2].display, displays[1]);
        store.save(&config)?;
        let reordered = Store::new(&path, id(1), vec![id(3), id(2)], base)?
            .with_displays(vec![displays[1], displays[0]])?
            .load()?
            .unwrap();
        assert_eq!(reordered.placements[1].x, config.placements[2].x);
        assert_eq!(reordered.placements[2].display, displays[0]);
        Ok(())
    }

    #[test]
    fn topology_is_bound_to_identity_set_and_actual_base_observation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let vault = dir.path().join("profile.vault");
        let observed = Observed {
            monitor: 12,
            display: display(3840, 2160, 150, 0),
        };
        let store = Store::new(&vault, id(1), vec![id(2), id(3)], observed)?;
        assert!(store.load()?.is_none());
        let config = Config {
            placements: vec![
                Placement {
                    id: 0,
                    x: 0,
                    y: 0,
                    display: observed.display,
                },
                Placement {
                    id: 1,
                    x: -864,
                    y: 200,
                    display: display(1080, 1920, 125, 90),
                },
                Placement {
                    id: 2,
                    x: 2560,
                    y: 0,
                    display: display(1920, 1080, 100, 0),
                },
            ],
            portal: Mode::Reduced,
        };
        store.save(&config)?;
        let stored = fs::read(&store.path)?;
        let reloaded = Store::new(&vault, id(1), vec![id(3), id(2)], observed)?
            .load()?
            .unwrap();
        assert_eq!(reloaded.placements[1].display, config.placements[2].display);
        assert_eq!(reloaded.placements[2].x, -864);
        assert!(reloaded.portal == Mode::Reduced);
        assert!(
            Store::new(
                &vault,
                id(1),
                vec![id(2), id(3)],
                Observed {
                    monitor: 13,
                    ..observed
                }
            )?
            .load()?
            .is_none()
        );
        assert!(
            Store::new(&vault, id(1), vec![id(2)], observed)?
                .load()?
                .is_none()
        );
        assert!(
            Store::new(&vault, id(5), vec![id(2), id(3)], observed)?
                .load()?
                .is_none()
        );
        let mut bad = config.clone();
        bad.placements[1].x = -863;
        assert!(store.save(&bad).is_err());
        assert_eq!(stored, fs::read(&store.path)?);
        bad = config.clone();
        bad.placements[1].x = -865;
        assert!(store.save(&bad).is_err());
        for malformed in [
            String::from_utf8(stored.clone())?.replace("ILGEOM1", "ILGEOM2"),
            String::from_utf8(stored.clone())?.replace(" 125 90", " 0 90"),
            "x".repeat(MAX_BYTES as usize + 1),
        ] {
            fs::write(&store.path, malformed.as_bytes())?;
            assert!(store.load().is_err());
            assert_eq!(fs::read(&store.path)?, malformed.as_bytes());
        }
        Ok(())
    }
}
