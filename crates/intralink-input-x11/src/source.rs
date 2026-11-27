//! Window-scoped physical XKB input. No root input selection or global grab.
use anyhow::{Context, Result, ensure};
use intralink_core::input::{Button, Event};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
use xcb::{Connection, Extension, x, xkb};

use crate::{keyboard::Keyboard, native::require_local_x11};

const WIDTH: u16 = 800;
const HEIGHT: u16 = 440;
const LIMIT: usize = 128;

xcb::atoms_struct! {
    struct Atoms {
        wm_protocols => b"WM_PROTOCOLS" only_if_exists = false,
        wm_delete => b"WM_DELETE_WINDOW" only_if_exists = false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadEvent {
    Select(usize),
    Local,
    Input(Event),
}

#[derive(Debug, Default)]
pub struct Poll {
    pub ready: bool,
    pub interrupted: bool,
    pub closed: bool,
    pub events: Vec<PadEvent>,
}

/// Only events delivered to the approved window are converted. A lost focus
/// or pointer boundary is latched even if focus returns within the same poll.
pub struct Pad {
    conn: Connection,
    window: x::Window,
    gc: x::Gcontext,
    keyboard: Keyboard,
    keys: BTreeSet<u8>,
    buttons: BTreeSet<u8>,
    focused: bool,
    inside: bool,
    visible: bool,
    mapped: bool,
    closed: bool,
    width: u16,
    height: u16,
    position: Option<(u16, u16)>,
    title: String,
}

fn normalized(x: i16, y: i16, width: u16, height: u16) -> Option<(u16, u16)> {
    if width < 2 || height < 2 || x < 0 || y < 0 || x as u16 >= width || y as u16 >= height {
        return None;
    }
    let scale = |v: i16, n: u16| {
        ((u32::from(v as u16) * 65535 + u32::from(n - 1) / 2) / u32::from(n - 1)) as u16
    };
    Some((scale(x, width), scale(y, height)))
}

impl Pad {
    pub fn open() -> Result<Self> {
        Self::open_display(&require_local_x11()?)
    }

    fn open_display(display: &str) -> Result<Self> {
        let (conn, index) =
            Connection::connect_with_extensions(Some(display), &[Extension::Xkb], &[])?;
        let screen = conn
            .get_setup()
            .roots()
            .nth(index as usize)
            .context("Missing X11 screen")?;
        ensure!(
            screen.width_in_pixels() >= WIDTH && screen.height_in_pixels() >= HEIGHT,
            "X11 display is too small for the input pad"
        );
        let version = conn.wait_for_reply(conn.send_request(&xkb::UseExtension {
            wanted_major: 1,
            wanted_minor: 0,
        }))?;
        ensure!(version.supported(), "XKB 1.0 required");
        let keyboard = Keyboard::read(&conn)?;
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
        // Use detectable repeat: a held key is not an artificial release/down
        // pair. Do not change the server's global repeat configuration.
        let repeat = conn.wait_for_reply(conn.send_request(&xkb::PerClientFlags {
            device_spec: xkb::Id::UseCoreKbd as xkb::DeviceSpec,
            change: xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT,
            value: xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT,
            ctrls_to_change: xkb::BoolCtrl::empty(),
            auto_ctrls: xkb::BoolCtrl::empty(),
            auto_ctrls_values: xkb::BoolCtrl::empty(),
        }))?;
        ensure!(
            repeat
                .value()
                .contains(xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT),
            "X11 detectable auto-repeat unavailable"
        );
        let atoms = Atoms::intern_all(&conn)?;
        let window: x::Window = conn.generate_id();
        let gc: x::Gcontext = conn.generate_id();
        conn.send_and_check_request(&x::CreateWindow {
            depth: x::COPY_FROM_PARENT as u8,
            wid: window,
            parent: screen.root(),
            x: 24,
            y: 24,
            width: WIDTH,
            height: HEIGHT,
            border_width: 1,
            class: x::WindowClass::InputOutput,
            visual: screen.root_visual(),
            value_list: &[
                x::Cw::BackPixel(screen.black_pixel()),
                x::Cw::BorderPixel(screen.white_pixel()),
                x::Cw::EventMask(
                    x::EventMask::EXPOSURE
                        | x::EventMask::VISIBILITY_CHANGE
                        | x::EventMask::STRUCTURE_NOTIFY
                        | x::EventMask::FOCUS_CHANGE
                        | x::EventMask::ENTER_WINDOW
                        | x::EventMask::LEAVE_WINDOW
                        | x::EventMask::KEY_PRESS
                        | x::EventMask::KEY_RELEASE
                        | x::EventMask::BUTTON_PRESS
                        | x::EventMask::BUTTON_RELEASE
                        | x::EventMask::POINTER_MOTION,
                ),
            ],
        })?;
        conn.send_and_check_request(&x::ChangeProperty {
            mode: x::PropMode::Replace,
            window,
            property: atoms.wm_protocols,
            r#type: x::ATOM_ATOM,
            data: &[atoms.wm_delete],
        })?;
        conn.send_and_check_request(&x::CreateGc {
            cid: gc,
            drawable: x::Drawable::Window(window),
            value_list: &[
                x::Gc::Foreground(screen.white_pixel()),
                x::Gc::Background(screen.black_pixel()),
            ],
        })?;
        let mut pad = Self {
            conn,
            window,
            gc,
            keyboard,
            keys: BTreeSet::new(),
            buttons: BTreeSet::new(),
            focused: false,
            inside: false,
            visible: false,
            mapped: false,
            closed: false,
            width: WIDTH,
            height: HEIGHT,
            position: None,
            title: String::new(),
        };
        pad.set_title("IntraLink INPUT | LOCAL | connecting")?;
        pad.conn.send_and_check_request(&x::MapWindow { window })?;
        // A window manager can map a managed window asynchronously.
        let started = Instant::now();
        while !pad.mapped || !pad.visible {
            let output = pad.poll()?;
            ensure!(
                !output.closed && started.elapsed() < Duration::from_secs(2),
                "X11 input pad did not become visible"
            );
            if !pad.mapped || !pad.visible {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(pad)
    }

    pub fn set_title(&mut self, title: &str) -> Result<()> {
        ensure!(
            title.len() <= 180 && title.is_ascii(),
            "Invalid X11 pad title"
        );
        if self.title != title {
            self.conn.send_and_check_request(&x::ChangeProperty {
                mode: x::PropMode::Replace,
                window: self.window,
                property: x::ATOM_WM_NAME,
                r#type: x::ATOM_STRING,
                data: title.as_bytes(),
            })?;
            self.title = title.to_owned();
            self.paint()?;
        }
        Ok(())
    }

    fn paint(&self) -> Result<()> {
        self.conn.send_and_check_request(&x::ClearArea {
            exposures: false,
            window: self.window,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        })?;
        for (y, text) in [
            (32, self.title.as_bytes()),
            (
                62,
                b"F1-F8 select an approved host. F12 returns local. Escape closes." as &[u8],
            ),
            (
                92,
                b"Input is forwarded only while this window is focused and the pointer is inside."
                    as &[u8],
            ),
        ] {
            self.conn.send_and_check_request(&x::ImageText8 {
                drawable: x::Drawable::Window(self.window),
                gc: self.gc,
                x: 16,
                y,
                string: text,
            })?;
        }
        Ok(())
    }

    fn ready(&self) -> bool {
        self.focused && self.inside && self.visible && self.mapped && !self.closed
    }

    fn interrupt(&mut self, output: &mut Poll) {
        output.interrupted = true;
        output.events.clear();
        self.keys.clear();
        self.buttons.clear();
        self.position = None;
    }

    fn pointer(&mut self, x: i16, y: i16, output: &mut Poll) {
        let position = normalized(x, y, self.width, self.height);
        if position.is_none() {
            self.inside = false;
            self.interrupt(output);
        } else if self.ready() && self.position != position {
            self.position = position;
            if let Some((x, y)) = position {
                output.events.push(PadEvent::Input(Event::Move { x, y }));
            }
        }
    }

    fn key(&mut self, code: u8, down: bool, output: &mut Poll) {
        if !self.ready() {
            return;
        }
        let Some(key) = self.keyboard.key_for_code(code) else {
            // An unsupported physical key may be held across activation.
            // Track it locally, but never translate it into an arbitrary code.
            if down {
                self.keys.insert(code);
            } else {
                self.keys.remove(&code);
            }
            return;
        };
        if down {
            if !self.keys.insert(code) {
                return;
            }
        } else if !self.keys.remove(&code) {
            return;
        }
        match key.usage() {
            41 if down => self.closed = true,
            58..=65 if !down => output
                .events
                .push(PadEvent::Select(usize::from(key.usage() - 58))),
            69 if down => output.events.push(PadEvent::Local),
            41 | 58..=66 | 69 => {}
            _ => output
                .events
                .push(PadEvent::Input(Event::Key { key, down })),
        }
    }

    fn button(&mut self, detail: u8, down: bool, output: &mut Poll) {
        if !self.ready() {
            return;
        }
        if down {
            if !self.buttons.insert(detail) {
                return;
            }
        } else if !self.buttons.remove(&detail) {
            return;
        }
        match detail {
            1..=3 => {
                let button = match detail {
                    1 => Button::Left,
                    2 => Button::Middle,
                    _ => Button::Right,
                };
                output
                    .events
                    .push(PadEvent::Input(Event::Button { button, down }));
            }
            4..=7 if down => output.events.push(PadEvent::Input(Event::Wheel {
                horizontal: detail >= 6,
                steps: if detail == 4 || detail == 7 { 1 } else { -1 },
            })),
            _ => {}
        }
    }

    /// Recheck actual focus and pointer on the owned window. Never select
    /// global keyboard events; only the neutral check queries held-state bits.
    pub fn neutral(&self) -> Result<bool> {
        if !self.ready() || !self.keys.is_empty() || !self.buttons.is_empty() {
            return Ok(false);
        }
        let focus = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::GetInputFocus {}))?;
        if focus.focus() != self.window {
            return Ok(false);
        }
        let pointer = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::QueryPointer {
                window: self.window,
            }))?;
        if !pointer.same_screen()
            || normalized(pointer.win_x(), pointer.win_y(), self.width, self.height).is_none()
            || pointer.mask().bits() & 0x1f00 != 0
        {
            return Ok(false);
        }
        let keys = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::QueryKeymap {}))?;
        Ok(keys.keys().iter().all(|&byte| byte == 0))
    }

    pub fn poll(&mut self) -> Result<Poll> {
        let mut output = Poll::default();
        let mut drained = false;
        for _ in 0..LIMIT {
            let Some(event) = self.conn.poll_for_event()? else {
                drained = true;
                break;
            };
            match event {
                xcb::Event::X(x::Event::FocusIn(e)) if e.event() == self.window => {
                    self.focused = true
                }
                xcb::Event::X(x::Event::FocusOut(e)) if e.event() == self.window => {
                    self.focused = false;
                    self.interrupt(&mut output);
                }
                xcb::Event::X(x::Event::EnterNotify(e)) if e.event() == self.window => {
                    self.inside = true
                }
                xcb::Event::X(x::Event::LeaveNotify(e)) if e.event() == self.window => {
                    self.inside = false;
                    self.interrupt(&mut output);
                }
                xcb::Event::X(x::Event::VisibilityNotify(e)) if e.window() == self.window => {
                    self.visible = e.state() == x::Visibility::Unobscured;
                    if !self.visible {
                        self.interrupt(&mut output);
                    }
                }
                xcb::Event::X(x::Event::Expose(e)) if e.window() == self.window => self.paint()?,
                xcb::Event::X(x::Event::ConfigureNotify(e)) if e.window() == self.window => {
                    if self.width != e.width() || self.height != e.height() {
                        self.width = e.width();
                        self.height = e.height();
                        self.interrupt(&mut output);
                    }
                }
                xcb::Event::X(x::Event::KeyPress(e))
                    if e.event() == self.window && e.response_type() & 0x80 == 0 =>
                {
                    self.key(e.detail(), true, &mut output)
                }
                xcb::Event::X(x::Event::KeyRelease(e))
                    if e.event() == self.window && e.response_type() & 0x80 == 0 =>
                {
                    self.key(e.detail(), false, &mut output)
                }
                xcb::Event::X(x::Event::MotionNotify(e))
                    if e.event() == self.window && e.response_type() & 0x80 == 0 =>
                {
                    self.pointer(e.event_x(), e.event_y(), &mut output)
                }
                xcb::Event::X(x::Event::ButtonPress(e))
                    if e.event() == self.window && e.response_type() & 0x80 == 0 =>
                {
                    self.pointer(e.event_x(), e.event_y(), &mut output);
                    self.button(e.detail(), true, &mut output);
                }
                xcb::Event::X(x::Event::ButtonRelease(e))
                    if e.event() == self.window && e.response_type() & 0x80 == 0 =>
                {
                    self.pointer(e.event_x(), e.event_y(), &mut output);
                    self.button(e.detail(), false, &mut output);
                }
                xcb::Event::X(x::Event::UnmapNotify(e)) if e.window() == self.window => {
                    self.closed = true
                }
                xcb::Event::X(x::Event::DestroyNotify(e)) if e.window() == self.window => {
                    self.closed = true
                }
                xcb::Event::X(x::Event::ClientMessage(e)) if e.window() == self.window => {
                    self.closed = true
                }
                xcb::Event::X(x::Event::MappingNotify(_)) | xcb::Event::Xkb(_) => {
                    self.closed = true
                }
                _ => {}
            }
            if self.closed {
                break;
            }
        }
        if !drained && !self.closed {
            self.closed = true;
        }
        if !self.closed {
            // Pipeline independent queries and confirm actual focus.
            let focus_request = self.conn.send_request(&x::GetInputFocus {});
            let pointer_request = self.conn.send_request(&x::QueryPointer {
                window: self.window,
            });
            let attrs_request = self.conn.send_request(&x::GetWindowAttributes {
                window: self.window,
            });
            let focus = self.conn.wait_for_reply(focus_request)?;
            let pointer = self.conn.wait_for_reply(pointer_request)?;
            let attrs = self.conn.wait_for_reply(attrs_request)?;
            let focused = focus.focus() == self.window;
            let inside = pointer.same_screen()
                && normalized(pointer.win_x(), pointer.win_y(), self.width, self.height).is_some();
            let mapped = attrs.map_state() == x::MapState::Viewable;
            if (self.focused && !focused) || (self.inside && !inside) || (self.mapped && !mapped) {
                self.interrupt(&mut output);
            }
            if self.mapped && !mapped {
                self.closed = true;
            }
            self.focused = focused;
            self.inside = inside;
            self.mapped = mapped;
            if self.ready() && !output.interrupted {
                self.pointer(pointer.win_x(), pointer.win_y(), &mut output);
            }
        }
        output.ready = self.ready();
        output.closed = self.closed;
        if !output.ready || output.interrupted || output.closed {
            output.events.clear();
        }
        Ok(output)
    }

    pub fn position(&self) -> Option<(u16, u16)> {
        if self.ready() { self.position } else { None }
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        let _ = self.conn.send_and_check_request(&x::FreeGc { gc: self.gc });
        let _ = self.conn.send_and_check_request(&x::DestroyWindow {
            window: self.window,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "run only under a disposable authenticated Xvfb display"]
    fn native_xvfb_focused_source_and_boundary() -> Result<()> {
        use intralink_core::input::Key;
        use xcb::xtest;

        let display = std::env::var("DISPLAY")?;
        ensure!(
            std::env::var("INTRALINK_X11_TEST_DISPLAY").as_deref() == Ok(display.as_str()),
            "Native test must run on a dedicated Xvfb display"
        );
        let mut pad = Pad::open_display(&display)?;
        let (driver, index) =
            Connection::connect_with_extensions(Some(&display), &[Extension::Test], &[])?;
        let root = driver
            .get_setup()
            .roots()
            .nth(index as usize)
            .context("Missing test root")?
            .root();
        let version = driver.wait_for_reply(driver.send_request(&xtest::GetVersion {
            major_version: 2,
            minor_version: 2,
        }))?;
        ensure!(
            (version.major_version(), version.minor_version()) >= (2, 2),
            "XTEST unavailable"
        );
        let inject = |kind, detail, x, y| -> Result<()> {
            driver.send_and_check_request(&xtest::FakeInput {
                r#type: kind,
                detail,
                time: 0,
                root,
                root_x: x,
                root_y: y,
                deviceid: 0,
            })?;
            Ok(())
        };
        driver.send_and_check_request(&x::SetInputFocus {
            revert_to: x::InputFocus::PointerRoot,
            focus: pad.window,
            time: x::CURRENT_TIME,
        })?;
        inject(6, 0, 200, 200)?;
        let initial = pad.poll()?;
        assert!(initial.ready && !initial.closed);
        assert!(pad.neutral()?);
        let key = Key::new(4).unwrap();
        let a = pad.keyboard.code(key)?;
        let f1 = pad.keyboard.code(Key::new(58).unwrap())?;
        inject(2, f1, 0, 0)?;
        inject(3, f1, 0, 0)?;
        let select = pad.poll()?;
        assert!(select.events.contains(&PadEvent::Select(0)));
        assert!(
            !select
                .events
                .iter()
                .any(|e| matches!(e, PadEvent::Input(Event::Key { .. })))
        );
        assert!(pad.neutral()?);

        // A fast tap must preserve both transitions rather than collapse them
        // into an empty held-state snapshot.
        inject(2, a, 0, 0)?;
        inject(3, a, 0, 0)?;
        inject(4, 1, 0, 0)?;
        inject(5, 1, 0, 0)?;
        inject(4, 4, 0, 0)?;
        inject(5, 4, 0, 0)?;
        let events: Vec<_> = pad
            .poll()?
            .events
            .into_iter()
            .filter(|e| !matches!(e, PadEvent::Input(Event::Move { .. })))
            .collect();
        assert_eq!(
            events,
            vec![
                PadEvent::Input(Event::Key { key, down: true }),
                PadEvent::Input(Event::Key { key, down: false }),
                PadEvent::Input(Event::Button {
                    button: Button::Left,
                    down: true
                }),
                PadEvent::Input(Event::Button {
                    button: Button::Left,
                    down: false
                }),
                PadEvent::Input(Event::Wheel {
                    horizontal: false,
                    steps: 1
                }),
            ]
        );

        // Leaving and re-entering before the next poll cannot hide the
        // interruption or deliver an event from the same batch.
        inject(6, 0, 1000, 750)?;
        inject(6, 0, 200, 200)?;
        inject(2, a, 0, 0)?;
        inject(3, a, 0, 0)?;
        let interrupted = pad.poll()?;
        assert!(interrupted.interrupted && interrupted.events.is_empty());
        assert!(pad.neutral()?);
        driver.send_and_check_request(&x::UnmapWindow { window: pad.window })?;
        assert!(pad.poll()?.closed);
        Ok(())
    }

    #[test]
    fn normalized_pointer_rejects_outside_and_preserves_edges() {
        assert_eq!(normalized(0, 0, 800, 440), Some((0, 0)));
        assert_eq!(normalized(799, 439, 800, 440), Some((65535, 65535)));
        assert_eq!(normalized(-1, 0, 800, 440), None);
        assert_eq!(normalized(800, 0, 800, 440), None);
    }
}
