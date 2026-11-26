//! Explicit local driver discovery. No media capture, encoding or network activity.
use std::{
    fs,
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use libva::{Display, VAEntrypoint, VAProfile};

const MAX_RENDER_NODES: usize = 64;

fn is_render_node(name: &str) -> bool {
    name.strip_prefix("renderD")
        .and_then(|suffix| suffix.parse::<u32>().ok())
        .is_some_and(|number| name == format!("renderD{number}"))
}

fn render_nodes() -> Result<Vec<PathBuf>> {
    let mut nodes = Vec::new();
    for entry in fs::read_dir("/dev/dri").context("No accessible DRM directory /dev/dri")? {
        let entry = entry?;
        if entry.file_name().to_str().is_some_and(is_render_node)
            && entry.file_type()?.is_char_device()
        {
            ensure!(nodes.len() < MAX_RENDER_NODES, "Too many DRM render nodes");
            nodes.push(entry.path());
        }
    }
    nodes.sort();
    ensure!(!nodes.is_empty(), "No DRM render nodes available");
    Ok(nodes)
}

/// Only report profiles of codecs planned for the media pipeline. Do not infer
/// acceleration from the PCI vendor, and do not select an encoder automatically.
fn codec_name(profile: VAProfile::Type) -> Option<&'static str> {
    match profile {
        VAProfile::VAProfileH264ConstrainedBaseline
        | VAProfile::VAProfileH264Main
        | VAProfile::VAProfileH264High => Some("H.264"),
        VAProfile::VAProfileVP9Profile0
        | VAProfile::VAProfileVP9Profile1
        | VAProfile::VAProfileVP9Profile2
        | VAProfile::VAProfileVP9Profile3 => Some("VP9"),
        VAProfile::VAProfileAV1Profile0 | VAProfile::VAProfileAV1Profile1 => Some("AV1"),
        _ => None,
    }
}

fn entrypoint_name(entrypoint: VAEntrypoint::Type) -> Option<&'static str> {
    match entrypoint {
        VAEntrypoint::VAEntrypointVLD => Some("decode"),
        VAEntrypoint::VAEntrypointEncSlice => Some("encode"),
        VAEntrypoint::VAEntrypointEncSliceLP => Some("encode-low-power"),
        _ => None,
    }
}

fn probe(node: &Path) -> Result<usize> {
    let display = Display::open_drm_display(node)
        .map_err(|err| anyhow::anyhow!("VA-API initialization: {err:?}"))?;
    let vendor = display
        .query_vendor_string()
        .map_err(|err| anyhow::anyhow!("VA-API vendor query: {err}"))?;
    println!("Device: {}\nDriver: {vendor:?}", node.display());
    let profiles = display
        .query_config_profiles()
        .map_err(|err| anyhow::anyhow!("VA-API profile query: {err:?}"))?;
    let mut advertised = 0usize;
    for profile in profiles {
        let Some(codec) = codec_name(profile) else {
            continue;
        };
        let entrypoints = display.query_config_entrypoints(profile).map_err(|err| {
            anyhow::anyhow!(
                "VA-API entrypoint query failed for {codec} profile {profile:?}: {err:?}"
            )
        })?;
        for entrypoint in entrypoints {
            let Some(operation) = entrypoint_name(entrypoint) else {
                continue;
            };
            // Creating a driver config tests more than the advertisement alone.
            // It still does not allocate surfaces or process a frame.
            let config = display.create_config(Vec::new(), profile, entrypoint);
            let state = match config {
                Ok(_) => "configuration created".to_owned(),
                Err(err) => format!("configuration rejected: {err:?}"),
            };
            println!("  {codec} | profile {profile:?} | {operation} | advertised | {state}");
            advertised += 1;
        }
    }
    if advertised == 0 {
        println!("  No planned H.264, VP9 or AV1 entrypoints advertised.");
    }
    Ok(advertised)
}

pub fn report() -> Result<()> {
    let nodes = render_nodes()?;
    let mut queried = 0usize;
    let mut incomplete = 0usize;
    let mut advertised = 0usize;
    for node in nodes {
        match probe(&node) {
            Ok(count) => {
                queried += 1;
                advertised += count;
            }
            Err(err) => {
                incomplete += 1;
                eprintln!("{}: {err:#}", node.display());
            }
        }
    }
    println!(
        "Completed devices: {queried}; incomplete devices: {incomplete}; advertised entries on completed devices: {advertised}"
    );
    println!(
        "No frames processed. Hardware encoder/decoder integration is not available in this release."
    );
    if incomplete != 0 {
        bail!("VA-API report incomplete; inspect driver and device permissions");
    }
    ensure!(queried > 0, "No VA-API device queried");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_node_names_are_strict() {
        for valid in ["renderD128", "renderD129", "renderD256"] {
            assert!(is_render_node(valid));
        }
        for invalid in [
            "card0",
            "renderD",
            "renderD+128",
            "renderD0128",
            "renderD-1",
            "renderD128.tmp",
            "renderD4294967296",
        ] {
            assert!(!is_render_node(invalid));
        }
    }

    #[test]
    fn planned_profiles_are_not_vendor_assumptions() {
        assert_eq!(codec_name(VAProfile::VAProfileH264Main), Some("H.264"));
        assert_eq!(codec_name(VAProfile::VAProfileVP9Profile0), Some("VP9"));
        assert_eq!(codec_name(VAProfile::VAProfileAV1Profile0), Some("AV1"));
        assert_eq!(codec_name(VAProfile::VAProfileMPEG2Main), None);
        assert_eq!(
            entrypoint_name(VAEntrypoint::VAEntrypointVLD),
            Some("decode")
        );
        assert_eq!(
            entrypoint_name(VAEntrypoint::VAEntrypointEncSlice),
            Some("encode")
        );
        assert_eq!(
            entrypoint_name(VAEntrypoint::VAEntrypointEncSliceLP),
            Some("encode-low-power")
        );
        assert_eq!(entrypoint_name(VAEntrypoint::VAEntrypointVideoProc), None);
    }
}
