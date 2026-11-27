//! Checked XTEST requests on a single explicitly approved local X11 desktop.
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use intralink_core::input::{Button, Event, Key, Record, State};
use xcb::{Connection, Extension, randr, x, xkb, xtest};

use crate::{indicator::Indicator, keyboard::Keyboard};

const REFRESH: Duration = Duration::from_millis(250);

fn local_display(display: &str) -> bool {
    let Some(suffix) = display.strip_prefix(':') else {
        return false;
    };
    let mut parts = suffix.split('.');
    let numeric = |s: &str| !s.is_empty() && s.len() <= 5 && s.bytes().all(|b| b.is_ascii_digit());
    parts.next().is_some_and(numeric) && parts.next().is_none_or(numeric) && parts.next().is_none()
}

fn require_local_x11() -> Result<String> {
    ensure!(
        std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("x11"),
        "Use an actual local X11 login session"
    );
    ensure!(
        std::env::var_os("WAYLAND_DISPLAY").is_none(),
        "Wayland/XWayland input is not supported"
    );
    ensure!(
        std::env::var_os("SSH_CONNECTION").is_none() && std::env::var_os("SSH_TTY").is_none(),
        "Remote shell input hosts are not supported"
    );
    let display = std::env::var("DISPLAY").context("No local X11 display")?;
    ensure!(
        local_display(&display),
        "Only a local X11 display is supported"
    );
    Ok(display)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Geometry {
    width: u16,
    height: u16,
}

impl Geometry {
    fn read(conn: &Connection, root: x::Window) -> Result<Self> {
        let root_size = conn.wait_for_reply(conn.send_request(&x::GetGeometry {
            drawable: x::Drawable::Window(root),
        }))?;
        let reply = conn.wait_for_reply(conn.send_request(&randr::GetMonitors {
            window: root,
            get_active: true,
        }))?;
        let mut monitors = reply.monitors();
        let monitor = monitors.next().context("No active X11 monitor")?;
        ensure!(
            monitors.next().is_none()
                && monitor.x() == 0
                && monitor.y() == 0
                && monitor.width() == root_size.width()
                && monitor.height() == root_size.height(),
            "X11 input preview requires exactly one monitor covering the root display"
        );
        let width = root_size.width();
        let height = root_size.height();
        ensure!(
            (500..=32767).contains(&width) && (160..=32767).contains(&height),
            "Unsupported X11 display dimensions"
        );
        Ok(Self { width, height })
    }

    fn point(self, x: u16, y: u16) -> (i16, i16) {
        fn scale(position: u16, size: u16) -> i16 {
            ((u32::from(position) * u32::from(size - 1) + 32767) / 65535) as i16
        }
        (scale(x, self.width), scale(y, self.height))
    }
}

fn button_number(button: Button) -> u8 {
    match button {
        Button::Left => 1,
        Button::Middle => 2,
        Button::Right => 3,
    }
}

fn wheel_number(horizontal: bool, steps: i16) -> u8 {
    match (horizontal, steps > 0) {
        (false, true) => 4,
        (false, false) => 5,
        (true, true) => 7,
        (true, false) => 6,
    }
}

/// Open only after local interactive approval. The stop window must remain
/// visible. No global grabs, privilege escalation, display selection from peers,
/// or access to other login sessions.
pub struct NativeInput {
    conn: Connection,
    root: x::Window,
    indicator: Indicator,
    keyboard: Keyboard,
    geometry: Geometry,
    state: State,
    keys: BTreeMap<Key, u8>,
    buttons: BTreeSet<u8>,
    last_check: Instant,
    failed: bool,
}

impl NativeInput {
    pub fn open() -> Result<Self> {
        Self::open_display(&require_local_x11()?)
    }

    fn open_display(display: &str) -> Result<Self> {
        let (conn, screen_num) = Connection::connect_with_extensions(
            Some(display),
            &[Extension::RandR, Extension::Xkb, Extension::Test],
            &[],
        )?;
        let setup = conn.get_setup();
        ensure!(
            setup.roots().count() == 1,
            "Multiple X11 screens are unsupported"
        );
        let screen = setup
            .roots()
            .nth(screen_num as usize)
            .context("Missing X11 screen")?;
        let root = screen.root();
        let test = conn.wait_for_reply(conn.send_request(&xtest::GetVersion {
            major_version: 2,
            minor_version: 2,
        }))?;
        ensure!(
            (test.major_version(), test.minor_version()) >= (2, 2),
            "XTEST 2.2 required"
        );
        let randr = conn.wait_for_reply(conn.send_request(&randr::QueryVersion {
            major_version: 1,
            minor_version: 5,
        }))?;
        ensure!(
            (randr.major_version(), randr.minor_version()) >= (1, 5),
            "RandR 1.5 required"
        );
        let xkb = conn.wait_for_reply(conn.send_request(&xkb::UseExtension {
            wanted_major: 1,
            wanted_minor: 0,
        }))?;
        ensure!(xkb.supported(), "XKB 1.0 required");
        let geometry = Geometry::read(&conn, root)?;
        let keyboard = Keyboard::read(&conn)?;
        let map = conn.wait_for_reply(conn.send_request(&x::GetPointerMapping {}))?;
        ensure!(
            map.map().starts_with(&[1, 2, 3, 4, 5, 6, 7]),
            "Unsupported X11 pointer mapping"
        );
        let changes = xkb::EventType::NEW_KEYBOARD_NOTIFY
            | xkb::EventType::MAP_NOTIFY
            | xkb::EventType::NAMES_NOTIFY;
        conn.send_and_check_request(&xkb::SelectEvents {
            device_spec: xkb::Id::UseCoreKbd as xkb::DeviceSpec,
            affect_which: changes,
            clear: xkb::EventType::empty(),
            select_all: changes,
            affect_map: xkb::MapPart::all(),
            map: xkb::MapPart::all(),
            details: &[],
        })?;
        conn.send_and_check_request(&randr::SelectInput {
            window: root,
            enable: randr::NotifyMask::SCREEN_CHANGE
                | randr::NotifyMask::CRTC_CHANGE
                | randr::NotifyMask::OUTPUT_CHANGE,
        })?;
        let indicator = Indicator::open(&conn, screen)?;
        Ok(Self {
            conn,
            root,
            indicator,
            keyboard,
            geometry,
            state: State::default(),
            keys: BTreeMap::new(),
            buttons: BTreeSet::new(),
            last_check: Instant::now(),
            failed: false,
        })
    }

    pub fn poll(&mut self) -> Result<bool> {
        if self.failed || !self.indicator.poll(&self.conn)? {
            self.failed = true;
            return Ok(false);
        }
        if self.last_check.elapsed() >= REFRESH {
            let geometry = Geometry::read(&self.conn, self.root)?;
            let keyboard = Keyboard::read(&self.conn)?;
            let map = self
                .conn
                .wait_for_reply(self.conn.send_request(&x::GetPointerMapping {}))?;
            if geometry != self.geometry
                || keyboard != self.keyboard
                || !map.map().starts_with(&[1, 2, 3, 4, 5, 6, 7])
            {
                self.failed = true;
                return Ok(false);
            }
            self.last_check = Instant::now();
        }
        Ok(true)
    }

    fn submit(&self, kind: u8, detail: u8, x: i16, y: i16) -> Result<()> {
        // A checked XCB request reports server protocol rejection. This is not
        // proof that the focused application acted on the event.
        self.conn.send_and_check_request(&xtest::FakeInput {
            r#type: kind,
            detail,
            time: 0,
            root: self.root,
            root_x: x,
            root_y: y,
            deviceid: 0,
        })?;
        Ok(())
    }

    fn key_down(&self, code: u8) -> Result<bool> {
        let reply = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::QueryKeymap {}))?;
        Ok(reply.keys()[usize::from(code) / 8] & (1 << (code % 8)) != 0)
    }

    fn button_down(&self, button: u8) -> Result<bool> {
        let pointer = self.conn.wait_for_reply(
            self.conn
                .send_request(&x::QueryPointer { window: self.root }),
        )?;
        ensure!(
            pointer.same_screen(),
            "Pointer is not on the approved X11 display"
        );
        Ok(pointer.mask().bits() & (1 << (7 + button)) != 0)
    }

    fn apply_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Activate | Event::Heartbeat => {}
            Event::Deactivate | Event::End => self.release_all()?,
            Event::Move { x, y } => {
                let (x, y) = self.geometry.point(x, y);
                self.submit(6, 0, x, y)?;
            }
            Event::Key { key, down } => {
                let code = self.keyboard.code(key)?;
                if down {
                    ensure!(
                        !self.key_down(code)?,
                        "Key already held locally; input stopped"
                    );
                    // Track a possibly accepted down before sending. Cleanup
                    // retries that exact code if the server reply is uncertain.
                    self.keys.insert(key, code);
                    self.submit(2, code, 0, 0)?;
                } else {
                    self.submit(3, code, 0, 0)?;
                    self.keys.remove(&key);
                }
            }
            Event::Button { button, down } => {
                let code = button_number(button);
                if down {
                    ensure!(
                        !self.button_down(code)?,
                        "Mouse button already held locally; input stopped"
                    );
                    self.buttons.insert(code);
                    self.submit(4, code, 0, 0)?;
                } else {
                    self.submit(5, code, 0, 0)?;
                    self.buttons.remove(&code);
                }
            }
            Event::Wheel { horizontal, steps } => {
                let code = wheel_number(horizontal, steps);
                for _ in 0..steps.unsigned_abs() {
                    ensure!(self.poll()?, "Local X11 input stop requested");
                    self.buttons.insert(code);
                    self.submit(4, code, 0, 0)?;
                    self.submit(5, code, 0, 0)?;
                    self.buttons.remove(&code);
                }
            }
        }
        Ok(())
    }

    pub fn apply(&mut self, record: Record) -> Result<()> {
        ensure!(!self.failed, "X11 input session already stopped");
        let result = (|| {
            ensure!(self.poll()?, "Local X11 stop window or display changed");
            self.state.validate(record)?;
            self.apply_event(record.event)?;
            self.state.commit(record)?;
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    pub fn release_all(&mut self) -> Result<()> {
        let mut failed = false;
        // HID ordering releases nonmodifiers before modifier keys; retain the
        // original X11 code even if the physical keymap changed.
        let keys: Vec<_> = self.keys.iter().map(|(&key, &code)| (key, code)).collect();
        let buttons: Vec<_> = self.buttons.iter().copied().collect();
        for (key, code) in keys {
            if self.submit(3, code, 0, 0).is_ok() {
                self.keys.remove(&key);
            } else {
                failed = true;
            }
        }
        for code in buttons {
            if self.submit(5, code, 0, 0).is_ok() {
                self.buttons.remove(&code);
            } else {
                failed = true;
            }
        }
        ensure!(!failed, "X11 rejected one or more input releases");
        Ok(())
    }
}

impl Drop for NativeInput {
    fn drop(&mut self) {
        if let Err(error) = self.release_all() {
            eprintln!("X11 input cleanup failed: {error:#}");
        }
        self.indicator.close(&self.conn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_display_policy_and_geometry() {
        for name in [":0", ":0.0", ":123.1"] {
            assert!(local_display(name));
        }
        for name in ["", "localhost:10.0", "host:0", "unix:0", ":0.0.1", ":abc"] {
            assert!(!local_display(name));
        }
        let geometry = Geometry {
            width: 1920,
            height: 1080,
        };
        assert_eq!(geometry.point(0, 0), (0, 0));
        assert_eq!(geometry.point(u16::MAX, u16::MAX), (1919, 1079));
        assert_eq!(geometry.point(32768, 32768), (960, 540));
        assert_eq!(button_number(Button::Right), 3);
        assert_eq!(wheel_number(true, 1), 7);
    }

    #[test]
    #[ignore = "run only under a disposable authenticated Xvfb display"]
    fn native_xvfb_input_and_release() -> Result<()> {
        let display = std::env::var("DISPLAY")?;
        ensure!(
            std::env::var("INTRALINK_X11_TEST_DISPLAY").as_deref() == Ok(display.as_str()),
            "Native test must run on a dedicated Xvfb display"
        );
        let mut native = NativeInput::open_display(&display)?;
        let key = Key::new(4).unwrap();
        let code = native.keyboard.code(key)?;
        let apply =
            |native: &mut NativeInput, sequence, event| native.apply(Record { sequence, event });
        apply(&mut native, 1, Event::Activate)?;
        apply(
            &mut native,
            2,
            Event::Move {
                x: u16::MAX,
                y: u16::MAX,
            },
        )?;
        let pointer = native
            .conn
            .wait_for_reply(native.conn.send_request(&x::QueryPointer {
                window: native.root,
            }))?;
        assert_eq!(
            (pointer.root_x(), pointer.root_y()),
            native.geometry.point(u16::MAX, u16::MAX)
        );
        apply(&mut native, 3, Event::Key { key, down: true })?;
        assert!(native.key_down(code)?);
        apply(
            &mut native,
            4,
            Event::Button {
                button: Button::Left,
                down: true,
            },
        )?;
        assert!(native.button_down(1)?);
        apply(&mut native, 5, Event::Deactivate)?;
        assert!(!native.key_down(code)? && !native.button_down(1)?);
        apply(&mut native, 6, Event::Activate)?;
        apply(&mut native, 7, Event::Key { key, down: true })?;
        native.conn.send_and_check_request(&x::UnmapWindow {
            window: native.indicator.window,
        })?;
        assert!(!native.poll()?);
        assert!(apply(&mut native, 8, Event::Move { x: 0, y: 0 }).is_err());
        native.release_all()?;
        assert!(!native.key_down(code)?);
        Ok(())
    }
}
