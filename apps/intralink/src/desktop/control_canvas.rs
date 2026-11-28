//! Shared geometry, visual presentation and local-only controls for the
//! unified viewer. Render at 1920x1080; chrome uses the existing bitmap font.
use super::super::{overview::text, resize::fitted};
use anyhow::{Result, ensure};
use intralink_core::{
    display_geometry::Display,
    overview::{self, Rect},
    presentation::Snapshot,
};

pub(super) const WIDTH: usize = 1920;
pub(super) const HEIGHT: usize = 1080;
const LW: usize = overview::WIDTH;
const LH: usize = overview::HEIGHT;
const BODY_TOP: usize = 100;
const BODY_BOTTOM: usize = 510;
const BODY: Rect = Rect {
    x: 24,
    y: BODY_TOP * 2,
    w: WIDTH - 48,
    h: (BODY_BOTTOM - BODY_TOP) * 2,
};
const BG: u32 = 0x101923;
const CARD: u32 = 0x223245;
const INK: u32 = 0xeff5fa;
const MUTED: u32 = 0xa8b9ca;
const ACCENT: u32 = 0x63ddc2;
const WARN: u32 = 0xffbd70;
const DANGER: u32 = 0x963747;
const BUTTON: u32 = 0x08685c;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lane {
    Idle,
    Connecting,
    Live,
    Stopping,
    Ended,
    Failed,
}
impl Lane {
    pub fn open(self) -> bool {
        matches!(self, Self::Connecting | Self::Live)
    }
    pub fn can_start(self) -> bool {
        matches!(self, Self::Idle | Self::Ended | Self::Failed)
    }
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "OFF",
            Self::Connecting => "CONNECTING",
            Self::Live => "CONNECTED",
            Self::Stopping => "STOPPING",
            Self::Ended => "ENDED",
            Self::Failed => "FAILED",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InputState {
    Safe,
    Waiting,
    Active,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Screen,
    Input,
    Verify,
    Arm,
    Close,
}
const SCREEN: Rect = Rect {
    x: 12,
    y: 57,
    w: 184,
    h: 34,
};
const INPUT: Rect = Rect {
    x: 206,
    y: 57,
    w: 184,
    h: 34,
};
const VERIFY: Rect = Rect {
    x: 400,
    y: 57,
    w: 202,
    h: 34,
};
const ARM: Rect = Rect {
    x: 612,
    y: 57,
    w: 214,
    h: 34,
};
const CLOSE: Rect = Rect {
    x: 836,
    y: 57,
    w: 112,
    h: 34,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Chrome {
    pub screen: Lane,
    pub input: Lane,
    pub state: InputState,
    pub verified: bool,
    pub verify_pending: bool,
    pub verify_allowed: bool,
    pub arm_allowed: bool,
    pub stale: bool,
    pub display: Option<Display>,
}
impl Chrome {
    pub fn enabled(self, action: Action) -> bool {
        match action {
            Action::Screen => self.screen.open() || self.screen.can_start(),
            Action::Input => self.input.open() || self.input.can_start(),
            Action::Verify => self.verify_allowed && self.state == InputState::Safe,
            Action::Arm => self.arm_allowed || self.state != InputState::Safe,
            Action::Close => true,
        }
    }
    pub fn hit(self, point: Option<(usize, usize)>) -> Option<Action> {
        let (x, y) = point?;
        let (x, y) = (x / 2, y / 2);
        [
            (SCREEN, Action::Screen),
            (INPUT, Action::Input),
            (VERIFY, Action::Verify),
            (ARM, Action::Arm),
            (CLOSE, Action::Close),
        ]
        .into_iter()
        .find(|(r, a)| r.contains(x, y) && self.enabled(*a))
        .map(|(_, a)| a)
    }
}

/// Complete fresh focused click on the same enabled local control.
#[derive(Default)]
pub(super) struct Click {
    neutral: bool,
    down: bool,
    pressed: Option<Action>,
}
impl Click {
    pub fn step(&mut self, focused: bool, down: bool, hit: Option<Action>) -> Option<Action> {
        if !focused {
            *self = Self::default();
            return None;
        }
        if !self.neutral {
            self.neutral = !down;
            self.down = down;
            return None;
        }
        let result = if down && !self.down {
            self.pressed = hit;
            None
        } else if !down && self.down {
            self.pressed.take().filter(|p| Some(*p) == hit)
        } else {
            None
        };
        self.down = down;
        result
    }
}

fn fit(rect: Rect, dimensions: (usize, usize)) -> Option<Rect> {
    let (w, h) = dimensions;
    if !(2..=16384).contains(&w) || !(2..=16384).contains(&h) {
        return None;
    }
    let (w, h) = fitted((w as u32, h as u32), (rect.w as u32, rect.h as u32));
    let (w, h) = (w as usize, h as usize);
    if w < 2 || h < 2 {
        return None;
    }
    Some(Rect {
        x: rect.x + (rect.w - w) / 2,
        y: rect.y + (rect.h - h) / 2,
        w,
        h,
    })
}

pub(super) fn map_window(size: (usize, usize), point: (f32, f32)) -> Option<(usize, usize)> {
    let (w, h) = (size.0 as f64, size.1 as f64);
    if w < 2.0 || h < 2.0 || !point.0.is_finite() || !point.1.is_finite() {
        return None;
    }
    let scale = (w / WIDTH as f64).min(h / HEIGHT as f64);
    let x = (f64::from(point.0) - (w - WIDTH as f64 * scale) / 2.0) / scale;
    let y = (f64::from(point.1) - (h - HEIGHT as f64 * scale) / 2.0) / scale;
    if !(0.0..WIDTH as f64).contains(&x) || !(0.0..HEIGHT as f64).contains(&y) {
        return None;
    }
    Some((x as usize, y as usize))
}

pub(super) fn image_rect(dimensions: (usize, usize)) -> Option<Rect> {
    fit(BODY, dimensions)
}

/// The screen stream contains host-side letterboxing. Project those exact
/// content boundaries through the nearest-neighbor presentation mapping.
/// This is valid only after explicit same-primary review; the existing
/// screen handshake itself does not attest monitor identity.
pub(super) fn video_region(frame: (usize, usize), display: Display) -> Option<Rect> {
    display.logical()?;
    if frame.0 > 1920 || frame.1 > 1080 {
        return None;
    }
    let image = image_rect(frame)?;
    let (w, h) = fitted(
        (u32::from(display.width), u32::from(display.height)),
        (frame.0 as u32, frame.1 as u32),
    );
    let (w, h) = (w as usize, h as usize);
    let left = (frame.0 - w) / 2;
    let top = (frame.1 - h) / 2;
    let x0 = (left * image.w).div_ceil(frame.0);
    let x1 = ((left + w) * image.w).div_ceil(frame.0);
    let y0 = (top * image.h).div_ceil(frame.1);
    let y1 = ((top + h) * image.h).div_ceil(frame.1);
    if x1 - x0 < 2 || y1 - y0 < 2 {
        return None;
    }
    Some(Rect {
        x: image.x + x0,
        y: image.y + y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}
pub(super) fn input_region(display: Display) -> Option<Rect> {
    display.logical()?;
    fit(
        BODY,
        (usize::from(display.width), usize::from(display.height)),
    )
}
pub(super) fn normalized(rect: Rect, point: (usize, usize)) -> Option<(u16, u16)> {
    if rect.w < 2 || rect.h < 2 || !rect.contains(point.0, point.1) {
        return None;
    }
    let map = |p: usize, n: usize| ((p * 65535 + (n - 1) / 2) / (n - 1)) as u16;
    Some((map(point.0 - rect.x, rect.w), map(point.1 - rect.y, rect.h)))
}

fn label(canvas: &mut [u32], x: usize, y: usize, value: &str, scale: usize, color: u32) {
    if x < LW && y < LH {
        text(canvas, (x, y), value, scale, (LW - x) / (6 * scale), color);
    }
}
fn button(canvas: &mut [u32], rect: Rect, value: &str, enabled: bool, danger: bool) {
    overview::fill(
        canvas,
        rect,
        if !enabled {
            0x35404e
        } else if danger {
            DANGER
        } else {
            BUTTON
        },
    );
    let n = value.chars().count().min(rect.w / 12);
    text(
        canvas,
        (rect.x + (rect.w - n * 12) / 2, rect.y + (rect.h - 14) / 2),
        value,
        2,
        n,
        INK,
    );
}
fn lane_color(lane: Lane) -> u32 {
    if lane == Lane::Live {
        ACCENT
    } else if lane == Lane::Failed {
        WARN
    } else {
        MUTED
    }
}

pub(super) struct Canvas {
    pub pixels: Vec<u32>,
    chrome: Vec<u32>,
    image: Option<((usize, usize), Rect)>,
    xmap: Vec<usize>,
}
impl Canvas {
    pub fn new() -> Self {
        Self {
            pixels: vec![BG; WIDTH * HEIGHT],
            chrome: vec![BG; LW * LH],
            image: None,
            xmap: Vec::new(),
        }
    }
    fn copy_rows(&mut self, from: usize, to: usize) {
        for y in from..to {
            let (first, rest) = self.pixels[y * 2 * WIDTH..(y * 2 + 2) * WIDTH].split_at_mut(WIDTH);
            for (color, pair) in self.chrome[y * LW..(y + 1) * LW]
                .iter()
                .zip(first.chunks_exact_mut(2))
            {
                pair.fill(*color);
            }
            rest.copy_from_slice(first);
        }
    }
    pub fn chrome(&mut self, name: &str, state: Chrome) {
        self.chrome.fill(BG);
        label(&mut self.chrome, 12, 12, "INTRALINK / CONTROL", 2, ACCENT);
        label(&mut self.chrome, 12, 34, name, 1, INK);
        label(
            &mut self.chrome,
            268,
            13,
            &format!("SCREEN / {}", state.screen.label()),
            1,
            lane_color(state.screen),
        );
        label(
            &mut self.chrome,
            268,
            30,
            &format!("INPUT / {}", state.input.label()),
            1,
            lane_color(state.input),
        );
        if let Some(d) = state.display {
            label(
                &mut self.chrome,
                562,
                13,
                &format!("PRIMARY {}X{} / {}%", d.width, d.height, d.scale),
                1,
                MUTED,
            );
        }
        label(
            &mut self.chrome,
            562,
            31,
            match state.state {
                InputState::Safe => "SAFE / LOCAL",
                InputState::Waiting => "ARMED / ENTER PAD",
                InputState::Active => "REMOTE INPUT ACTIVE",
            },
            2,
            if state.state == InputState::Active {
                WARN
            } else {
                ACCENT
            },
        );
        button(
            &mut self.chrome,
            SCREEN,
            if state.screen.open() {
                "STOP SCREEN"
            } else {
                "START SCREEN"
            },
            state.enabled(Action::Screen),
            state.screen.open(),
        );
        button(
            &mut self.chrome,
            INPUT,
            if state.input.open() {
                "STOP INPUT"
            } else {
                "START INPUT"
            },
            state.enabled(Action::Input),
            state.input.open(),
        );
        button(
            &mut self.chrome,
            VERIFY,
            if state.verified {
                "PRIMARY MATCHED"
            } else if state.verify_pending {
                "CONFIRM MATCH"
            } else {
                "VERIFY PRIMARY"
            },
            state.enabled(Action::Verify),
            false,
        );
        button(
            &mut self.chrome,
            ARM,
            if state.state == InputState::Safe {
                "ARM INPUT"
            } else {
                "RELEASE INPUT"
            },
            state.enabled(Action::Arm),
            state.state != InputState::Safe,
        );
        button(&mut self.chrome, CLOSE, "CLOSE", true, true);
        let footer = if state.stale {
            "LAST FRAME / NO NEW NATIVE SAMPLE - INPUT SAFE"
        } else if state.verify_pending {
            "CONFIRM ONLY IF YOU VERIFIED THIS VIDEO IS THE CLIENT PRIMARY DISPLAY"
        } else if state.screen == Lane::Live && !state.verified {
            "VERIFY THAT VIDEO SHOWS THE CLIENT PRIMARY DISPLAY BEFORE ARMING"
        } else if state.state == InputState::Active {
            "F12 RELEASES / LEAVING THE CONTENT OR FOCUS RELEASES"
        } else {
            "SEPARATE CLIENT APPROVALS / F12 RELEASES / ESC CLOSES"
        };
        label(
            &mut self.chrome,
            12,
            520,
            footer,
            1,
            if state.stale { WARN } else { MUTED },
        );
        self.copy_rows(0, BODY_TOP);
        self.copy_rows(BODY_BOTTOM, LH);
    }

    fn clear_body(&mut self) {
        self.pixels[BODY_TOP * 2 * WIDTH..BODY_BOTTOM * 2 * WIDTH].fill(0x111c28);
    }

    pub fn placeholder(&mut self, screen: Lane, display: Option<Display>) {
        self.image = None;
        self.clear_body();
        for y in BODY_TOP..BODY_BOTTOM {
            self.chrome[y * LW..(y + 1) * LW].fill(0x111c28);
        }
        if screen.can_start()
            && let Some(rect) = display.and_then(input_region)
        {
            let logical = Rect {
                x: rect.x / 2,
                y: rect.y / 2,
                w: rect.w / 2,
                h: rect.h / 2,
            };
            overview::fill(&mut self.chrome, logical, CARD);
            if logical.w >= 132 && logical.h >= 45 {
                label(
                    &mut self.chrome,
                    logical.x + 8,
                    logical.y + 12,
                    "INPUT PAD / NO VIDEO",
                    1,
                    ACCENT,
                );
                label(
                    &mut self.chrome,
                    logical.x + 8,
                    logical.y + 30,
                    "ARM INPUT TO USE",
                    1,
                    INK,
                );
            }
        } else {
            let status = if screen == Lane::Connecting {
                "WAITING FOR CLIENT SCREEN APPROVAL"
            } else if screen == Lane::Stopping {
                "STOPPING SCREEN"
            } else {
                "SCREEN OFF / START SCREEN OR INPUT"
            };
            label(&mut self.chrome, 220, 280, status, 2, MUTED);
        }
        self.copy_rows(BODY_TOP, BODY_BOTTOM);
        // Correct the at-most-one-pixel odd-coordinate border introduced
        // by the 2x chrome grid so the visible pad equals the hit region.
        if screen.can_start()
            && let Some(rect) = display.and_then(input_region)
        {
            let (x0, y0, x1, y1) = (rect.x, rect.y, rect.x + rect.w, rect.y + rect.h);
            for y in y0.saturating_sub(1)..(y1 + 1).min(BODY_BOTTOM * 2) {
                if y < BODY_TOP * 2 {
                    continue;
                }
                for x in x0.saturating_sub(1)..(x1 + 1).min(WIDTH) {
                    if y == y0 || y == y1 - 1 || x == x0 || x == x1 - 1 {
                        self.pixels[y * WIDTH + x] = if rect.contains(x, y) {
                            ACCENT
                        } else {
                            0x111c28
                        };
                    } else if (x < x0 || x >= x1 || y < y0 || y >= y1)
                        && self.pixels[y * WIDTH + x] == CARD
                    {
                        self.pixels[y * WIDTH + x] = 0x111c28;
                    }
                }
            }
        }
    }

    pub fn frame(&mut self, frame: &Snapshot) -> Result<()> {
        let (sw, sh) = frame.dimensions();
        let dst = image_rect((sw, sh)).ok_or_else(|| anyhow::anyhow!("Invalid video geometry"))?;
        ensure!(
            sw <= 1920 && sh <= 1080 && sw * sh == frame.pixels().len(),
            "Invalid decoded frame"
        );
        // Clear only the presentation body. UI and remote pixels never share
        // an input hit region; no intermediate full-size resize allocation.
        if self.image != Some(((sw, sh), dst)) {
            self.clear_body();
            self.xmap.clear();
            self.xmap.extend((0..dst.w).map(|x| x * sw / dst.w));
            self.image = Some(((sw, sh), dst));
        }
        for y in 0..dst.h {
            let src_y = y * sh / dst.h;
            let src = &frame.pixels()[src_y * sw..(src_y + 1) * sw];
            let row =
                &mut self.pixels[(dst.y + y) * WIDTH + dst.x..(dst.y + y) * WIDTH + dst.x + dst.w];
            if sw == dst.w {
                row.copy_from_slice(src);
            } else {
                for (pixel, x) in row.iter_mut().zip(&self.xmap) {
                    *pixel = src[*x];
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "control_canvas_tests.rs"]
mod tests;
