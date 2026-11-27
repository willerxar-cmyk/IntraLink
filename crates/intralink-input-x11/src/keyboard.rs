//! Resolve protocol HID positions against the X server's physical XKB names.
//! Never infer a physical position from a translated keysym.
use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use intralink_core::input::Key;
use xcb::{Connection, xkb};

fn position(key: Key) -> &'static str {
    let usage = key.usage();
    match usage {
        4..=29 => [
            "AC01", "AB05", "AB03", "AC03", "AD03", "AC04", "AC05", "AC06", "AD08", "AC07", "AC08",
            "AC09", "AB07", "AB06", "AD09", "AD10", "AD01", "AD04", "AC02", "AD05", "AD07", "AB04",
            "AD02", "AB02", "AD06", "AB01",
        ][(usage - 4) as usize],
        30..=39 => [
            "AE01", "AE02", "AE03", "AE04", "AE05", "AE06", "AE07", "AE08", "AE09", "AE10",
        ][(usage - 30) as usize],
        40 => "RTRN",
        41 => "ESC",
        42 => "BKSP",
        43 => "TAB",
        44 => "SPCE",
        45 => "AE11",
        46 => "AE12",
        47 => "AD11",
        48 => "AD12",
        49 => "BKSL",
        51 => "AC10",
        52 => "AC11",
        53 => "TLDE",
        54 => "AB08",
        55 => "AB09",
        56 => "AB10",
        57 => "CAPS",
        58..=69 => [
            "FK01", "FK02", "FK03", "FK04", "FK05", "FK06", "FK07", "FK08", "FK09", "FK10", "FK11",
            "FK12",
        ][(usage - 58) as usize],
        71 => "SCLK",
        73 => "INS",
        74 => "HOME",
        75 => "PGUP",
        76 => "DELE",
        77 => "END",
        78 => "PGDN",
        79 => "RGHT",
        80 => "LEFT",
        81 => "DOWN",
        82 => "UP",
        83 => "NMLK",
        84 => "KPDV",
        85 => "KPMU",
        86 => "KPSU",
        87 => "KPAD",
        88 => "KPEN",
        89..=97 => [
            "KP1", "KP2", "KP3", "KP4", "KP5", "KP6", "KP7", "KP8", "KP9",
        ][(usage - 89) as usize],
        98 => "KP0",
        99 => "KPDL",
        224 => "LCTL",
        225 => "LFSH",
        226 => "LALT",
        227 => "LWIN",
        228 => "RCTL",
        229 => "RTSH",
        230 => "RALT",
        231 => "RWIN",
        _ => unreachable!("Key restricts the supported HID subset"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Keyboard([Option<u8>; 256]);

impl Keyboard {
    pub fn read(conn: &Connection) -> Result<Self> {
        let reply = conn.wait_for_reply(conn.send_request(&xkb::GetNames {
            device_spec: xkb::Id::UseCoreKbd as xkb::DeviceSpec,
            which: xkb::NameDetail::KEY_NAMES,
        }))?;
        let first = reply.first_key();
        let mut table = [None; 256];
        let positions: BTreeMap<_, _> = (0..=u8::MAX)
            .filter_map(Key::new)
            .map(|key| (position(key), key.usage()))
            .collect();
        for value in reply.value_list() {
            if let xkb::GetNamesReplyValueList::KeyNames(key_names) = value {
                for (offset, name) in key_names.iter().enumerate() {
                    let code = usize::from(first) + offset;
                    ensure!(code <= 255, "Invalid XKB keycode range");
                    let bytes = name.name.as_bytes();
                    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                    let name = std::str::from_utf8(&bytes[..end]).unwrap_or("").trim_end();
                    if let Some(&usage) = positions.get(name) {
                        ensure!(
                            table[usage as usize].is_none(),
                            "Ambiguous XKB physical key"
                        );
                        table[usage as usize] = Some(code as u8);
                    }
                }
            }
        }
        // Refuse incomplete/unrecognized keymaps rather than falling back to
        // guessed numeric X11 codes. Every wire key needs a physical mapping.
        ensure!(
            (0..=u8::MAX)
                .filter(|&u| Key::new(u).is_some())
                .all(|u| table[u as usize].is_some()),
            "XKB physical keymap does not support the complete input subset"
        );
        Ok(Self(table))
    }

    pub fn code(&self, key: Key) -> Result<u8> {
        self.0[key.usage() as usize].context("Unsupported physical key")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_wire_key_has_one_distinct_physical_position() {
        let mut seen = BTreeSet::new();
        for usage in 0..=u8::MAX {
            if let Some(key) = Key::new(usage) {
                let name = position(key);
                assert!((2..=4).contains(&name.len()));
                assert!(seen.insert(name), "Duplicate mapping: {name}");
            }
        }
        assert_eq!(position(Key::new(4).unwrap()), "AC01");
        assert_eq!(position(Key::new(228).unwrap()), "RCTL");
    }
}
