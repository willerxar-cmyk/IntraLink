//! Windows input boundary; no hooks, elevation, services, or screen capture.
#![cfg(target_os = "windows")]

mod entry_portal;
mod foreground_capture;
pub use foreground_capture::{ForegroundCapture, neutral_for_handoff};
mod physical;
pub use entry_portal::{EntryPortal, PortalStyle};
pub use physical::{indicator_visible, physical_cursor, physical_primary, with_physical_dpi};

use anyhow::{Context, Result, ensure};
use intralink_core::input::{Button, Event, Key, Record, State};
use std::collections::BTreeSet;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
};

/// The supported USB HID usages map to set-1 physical scan codes. Layout
/// translation is performed by Windows at the destination, not by the peer.
fn scan(key: Key) -> Option<(u16, bool)> {
    let usage = key.usage();
    let normal = |code| Some((code, false));
    let extended = |code| Some((code, true));
    match usage {
        4..=29 => normal(
            [
                0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31,
                0x18, 0x19, 0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c,
            ][(usage - 4) as usize],
        ),
        30..=39 => normal(u16::from(usage - 28)),
        40 => normal(0x1c),
        41 => normal(0x01),
        42 => normal(0x0e),
        43 => normal(0x0f),
        44 => normal(0x39),
        45 => normal(0x0c),
        46 => normal(0x0d),
        47 => normal(0x1a),
        48 => normal(0x1b),
        49 => normal(0x2b),
        51 => normal(0x27),
        52 => normal(0x28),
        53 => normal(0x29),
        54 => normal(0x33),
        55 => normal(0x34),
        56 => normal(0x35),
        57 => normal(0x3a),
        58..=67 => normal(u16::from(usage - 58) + 0x3b),
        68 => normal(0x57),
        69 => normal(0x58),
        71 => normal(0x46),
        73 => extended(0x52),
        74 => extended(0x47),
        75 => extended(0x49),
        76 => extended(0x53),
        77 => extended(0x4f),
        78 => extended(0x51),
        79 => extended(0x4d),
        80 => extended(0x4b),
        81 => extended(0x50),
        82 => extended(0x48),
        83 => extended(0x45),
        84 => extended(0x35),
        85 => normal(0x37),
        86 => normal(0x4a),
        87 => normal(0x4e),
        88 => extended(0x1c),
        89..=97 => {
            normal([0x4f, 0x50, 0x51, 0x4b, 0x4c, 0x4d, 0x47, 0x48, 0x49][(usage - 89) as usize])
        }
        98 => normal(0x52),
        99 => normal(0x53),
        224 => normal(0x1d),
        225 => normal(0x2a),
        226 => normal(0x38),
        227 => extended(0x5b),
        228 => extended(0x1d),
        229 => normal(0x36),
        230 => extended(0x38),
        231 => extended(0x5c),
        _ => None,
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS, x: i32, y: i32, data: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: x,
                dy: y,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn native(event: Event) -> Result<Option<INPUT>> {
    Ok(Some(match event {
        Event::Key { key, down } => {
            let (code, extended) = scan(key).context("Unsupported native physical key")?;
            let mut flags = KEYEVENTF_SCANCODE;
            if extended {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            if !down {
                flags |= KEYEVENTF_KEYUP;
            }
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wScan: code,
                        dwFlags: flags,
                        ..Default::default()
                    },
                },
            }
        }
        Event::Move { x, y } => mouse(
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
            i32::from(x),
            i32::from(y),
            0,
        ),
        Event::Button { button, down } => mouse(
            match (button, down) {
                (Button::Left, true) => MOUSEEVENTF_LEFTDOWN,
                (Button::Left, false) => MOUSEEVENTF_LEFTUP,
                (Button::Right, true) => MOUSEEVENTF_RIGHTDOWN,
                (Button::Right, false) => MOUSEEVENTF_RIGHTUP,
                (Button::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
                (Button::Middle, false) => MOUSEEVENTF_MIDDLEUP,
            },
            0,
            0,
            0,
        ),
        Event::Wheel { horizontal, steps } => {
            ensure!(
                steps != 0 && (-8..=8).contains(&steps),
                "Invalid wheel steps"
            );
            mouse(
                if horizontal {
                    MOUSEEVENTF_HWHEEL
                } else {
                    MOUSEEVENTF_WHEEL
                },
                0,
                0,
                (i32::from(steps) * 120) as u32,
            )
        }
        Event::Activate | Event::Deactivate | Event::Heartbeat | Event::End => return Ok(None),
    }))
}

fn submit(event: Event) -> Result<()> {
    if let Some(input) = native(event)? {
        // SAFETY: exactly one initialized INPUT of the selected union variant
        // is borrowed for this synchronous system call. No privileged desktop
        // access or UIPI bypass is attempted; Windows may reject injection.
        let accepted = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        ensure!(
            accepted == 1,
            "Windows rejected input; no privilege escalation or fallback"
        );
    }
    Ok(())
}

/// Owned only by the explicitly approved, visible foreground Host. The wire
/// protocol is independently revalidated here. Drop attempts to release only
/// input successfully injected by this instance.
#[derive(Default)]
pub struct NativeInput {
    state: State,
    keys: BTreeSet<Key>,
    buttons: BTreeSet<Button>,
}

impl NativeInput {
    pub fn apply(&mut self, record: Record) -> Result<()> {
        self.state.validate(record)?;
        match record.event {
            Event::Deactivate | Event::End => self.release_all()?,
            event => {
                submit(event)?;
                match event {
                    Event::Key { key, down: true } => {
                        self.keys.insert(key);
                    }
                    Event::Key { key, down: false } => {
                        self.keys.remove(&key);
                    }
                    Event::Button { button, down: true } => {
                        self.buttons.insert(button);
                    }
                    Event::Button {
                        button,
                        down: false,
                    } => {
                        self.buttons.remove(&button);
                    }
                    _ => {}
                }
            }
        }
        self.state.commit(record)?;
        Ok(())
    }

    pub fn release_all(&mut self) -> Result<()> {
        let keys: Vec<_> = self.keys.iter().copied().collect();
        let buttons: Vec<_> = self.buttons.iter().copied().collect();
        let mut failed = false;
        for key in keys {
            if submit(Event::Key { key, down: false }).is_ok() {
                self.keys.remove(&key);
            } else {
                failed = true;
            }
        }
        for button in buttons {
            if submit(Event::Button {
                button,
                down: false,
            })
            .is_ok()
            {
                self.buttons.remove(&button);
            } else {
                failed = true;
            }
        }
        ensure!(!failed, "Windows rejected one or more owned input releases");
        Ok(())
    }
}

impl Drop for NativeInput {
    fn drop(&mut self) {
        if let Err(error) = self.release_all() {
            eprintln!("Input release cleanup failed: {error:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_accepted_key_has_a_physical_mapping_without_injection() -> Result<()> {
        for usage in 0..=u8::MAX {
            if let Some(key) = Key::new(usage) {
                assert!(scan(key).is_some(), "Missing mapping for {usage}");
                for down in [true, false] {
                    assert!(native(Event::Key { key, down })?.is_some());
                }
            }
        }
        assert_eq!(scan(Key::new(4).unwrap()), Some((0x1e, false)));
        assert_eq!(scan(Key::new(228).unwrap()), Some((0x1d, true)));
        assert!(native(Event::Heartbeat)?.is_none());
        assert!(
            native(Event::Wheel {
                horizontal: false,
                steps: 9
            })
            .is_err()
        );
        Ok(())
    }
}
