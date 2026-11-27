//! Local stop window, never bypasses the window manager or grabs input.
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use xcb::{Connection, x};

xcb::atoms_struct! {
    struct Atoms {
        wm_protocols => b"WM_PROTOCOLS",
        wm_delete => b"WM_DELETE_WINDOW",
        wm_state => b"_NET_WM_STATE",
        wm_above => b"_NET_WM_STATE_ABOVE",
    }
}

pub(crate) struct Indicator {
    pub window: x::Window,
    gc: x::Gcontext,
    visible: bool,
    stopped: bool,
}

impl Indicator {
    pub fn open(conn: &Connection, screen: &x::Screen) -> Result<Self> {
        let atoms = Atoms::intern_all(conn)?;
        let red = conn
            .wait_for_reply(conn.send_request(&x::AllocColor {
                cmap: screen.default_colormap(),
                red: 0xb924,
                green: 0x2436,
                blue: 0x3636,
            }))?
            .pixel();
        let window: x::Window = conn.generate_id();
        let gc: x::Gcontext = conn.generate_id();
        conn.send_and_check_request(&x::CreateWindow {
            depth: x::COPY_FROM_PARENT as u8,
            wid: window,
            parent: screen.root(),
            x: 24,
            y: 24,
            width: 460,
            height: 80,
            border_width: 2,
            class: x::WindowClass::InputOutput,
            visual: screen.root_visual(),
            value_list: &[
                x::Cw::BackPixel(red),
                x::Cw::BorderPixel(screen.white_pixel()),
                x::Cw::EventMask(
                    x::EventMask::EXPOSURE
                        | x::EventMask::VISIBILITY_CHANGE
                        | x::EventMask::STRUCTURE_NOTIFY
                        | x::EventMask::BUTTON_PRESS
                        | x::EventMask::KEY_PRESS,
                ),
            ],
        })?;
        conn.send_and_check_request(&x::ChangeProperty {
            mode: x::PropMode::Replace,
            window,
            property: x::ATOM_WM_NAME,
            r#type: x::ATOM_STRING,
            data: b"IntraLink INPUT SHARING - STOP",
        })?;
        conn.send_and_check_request(&x::ChangeProperty {
            mode: x::PropMode::Replace,
            window,
            property: atoms.wm_protocols,
            r#type: x::ATOM_ATOM,
            data: &[atoms.wm_delete],
        })?;
        // Ask the window manager to keep the stop window above normal windows.
        // If it cannot remain unobscured, the input session is stopped instead.
        conn.send_and_check_request(&x::ChangeProperty {
            mode: x::PropMode::Replace,
            window,
            property: atoms.wm_state,
            r#type: x::ATOM_ATOM,
            data: &[atoms.wm_above],
        })?;
        conn.send_and_check_request(&x::CreateGc {
            cid: gc,
            drawable: x::Drawable::Window(window),
            value_list: &[
                x::Gc::Foreground(screen.white_pixel()),
                x::Gc::Background(red),
            ],
        })?;
        let mut indicator = Self {
            window,
            gc,
            visible: false,
            stopped: false,
        };
        conn.send_and_check_request(&x::MapWindow { window })?;
        let start = Instant::now();
        while !indicator.visible && !indicator.stopped && start.elapsed() < Duration::from_secs(2) {
            indicator.drain(conn)?;
            if !indicator.visible {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        ensure!(
            indicator.visible && !indicator.stopped,
            "X11 stop window is not fully visible"
        );
        indicator.paint(conn)?;
        Ok(indicator)
    }

    fn paint(&self, conn: &Connection) -> Result<()> {
        conn.send_and_check_request(&x::ImageText8 {
            drawable: x::Drawable::Window(self.window),
            gc: self.gc,
            x: 16,
            y: 30,
            string: b"INTRALINK - REMOTE INPUT ACTIVE",
        })?;
        conn.send_and_check_request(&x::ImageText8 {
            drawable: x::Drawable::Window(self.window),
            gc: self.gc,
            x: 16,
            y: 54,
            string: b"CLICK THIS WINDOW OR PRESS A KEY HERE TO STOP",
        })?;
        Ok(())
    }

    fn drain(&mut self, conn: &Connection) -> Result<()> {
        // Bound the amount of event processing per iteration. An event flood
        // is an unsafe local state, not an opportunity to continue blindly.
        for _ in 0..128 {
            let Some(event) = conn.poll_for_event()? else {
                return Ok(());
            };
            match event {
                xcb::Event::X(x::Event::VisibilityNotify(event))
                    if event.window() == self.window =>
                {
                    self.visible = event.state() == x::Visibility::Unobscured;
                    if !self.visible {
                        self.stopped = true;
                    }
                }
                xcb::Event::X(x::Event::Expose(event)) if event.window() == self.window => {
                    self.paint(conn)?
                }
                xcb::Event::X(x::Event::ButtonPress(event)) if event.event() == self.window => {
                    self.stopped = true
                }
                xcb::Event::X(x::Event::KeyPress(event)) if event.event() == self.window => {
                    self.stopped = true
                }
                xcb::Event::X(x::Event::UnmapNotify(event)) if event.window() == self.window => {
                    self.stopped = true
                }
                xcb::Event::X(x::Event::DestroyNotify(event)) if event.window() == self.window => {
                    self.stopped = true
                }
                xcb::Event::X(x::Event::ClientMessage(event)) if event.window() == self.window => {
                    self.stopped = true
                }
                xcb::Event::X(x::Event::MappingNotify(_))
                | xcb::Event::RandR(_)
                | xcb::Event::Xkb(_) => {
                    self.stopped = true;
                }
                _ => {}
            }
            if self.stopped {
                return Ok(());
            }
        }
        self.stopped = true;
        Ok(())
    }

    pub fn poll(&mut self, conn: &Connection) -> Result<bool> {
        self.drain(conn)?;
        if self.stopped || !self.visible {
            return Ok(false);
        }
        let attributes = conn.wait_for_reply(conn.send_request(&x::GetWindowAttributes {
            window: self.window,
        }))?;
        if attributes.map_state() != x::MapState::Viewable {
            self.stopped = true;
        }
        Ok(!self.stopped)
    }

    pub fn close(&self, conn: &Connection) {
        let _ = conn.send_and_check_request(&x::FreeGc { gc: self.gc });
        let _ = conn.send_and_check_request(&x::DestroyWindow {
            window: self.window,
        });
    }
}
