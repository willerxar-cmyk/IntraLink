//! Local-only invitation review and QR rendering. No listener, clipboard,
//! camera, keyboard hook, vault write, or capture is owned by these windows.
use super::overview::text;
use anyhow::{Result, ensure};
use intralink_core::overview::{self, HEIGHT, Rect, WIDTH};
use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};
use qrcode::{Color, QrCode};
use std::time::Instant;

use crate::enrollment::{Review, ReviewTime, now};

const BG: u32 = 0x101923;
const SURFACE: u32 = 0x223245;
const INK: u32 = 0xeff5fa;
const MUTED: u32 = 0xa8b9ca;
const ACCENT: u32 = 0x63ddc2;
const APPROVE: Rect = Rect {
    x: 656,
    y: 465,
    w: 256,
    h: 48,
};
const CANCEL: Rect = Rect {
    x: 48,
    y: 465,
    w: 172,
    h: 48,
};
const VERIFIED: Rect = Rect {
    x: 48,
    y: 401,
    w: 864,
    h: 44,
};
const CLOSE: Rect = Rect {
    x: 518,
    y: 450,
    w: 394,
    h: 44,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Verify,
    Approve,
    Cancel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Held {
    #[default]
    None,
    Pointer,
    VerifyKey,
    ApproveKey,
    Mixed,
}

#[derive(Default)]
struct ReviewInput {
    verified: bool,
    neutral: bool,
    held: Held,
    pressed: Option<Action>,
}

impl ReviewInput {
    fn step(
        &mut self,
        focused: bool,
        down: bool,
        at: Option<Action>,
        space: bool,
        enter: bool,
    ) -> Option<Action> {
        if !focused {
            *self = Self::default();
            return None;
        }
        let held = match (down, space, enter) {
            (false, false, false) => Held::None,
            (true, false, false) => Held::Pointer,
            (false, true, false) => Held::VerifyKey,
            (false, false, true) => Held::ApproveKey,
            _ => Held::Mixed,
        };
        if !self.neutral {
            self.neutral = held == Held::None;
            self.held = held;
            return None;
        }
        let mut action = None;
        match (self.held, held) {
            (Held::None, Held::Pointer | Held::VerifyKey | Held::ApproveKey) => {
                let requested = match held {
                    Held::Pointer => at,
                    Held::VerifyKey => Some(Action::Verify),
                    Held::ApproveKey => Some(Action::Approve),
                    _ => None,
                };
                self.pressed = requested.filter(|a| *a != Action::Approve || self.verified);
            }
            (previous, Held::None) if previous != Held::None => {
                let released = match previous {
                    Held::Pointer => at,
                    Held::VerifyKey => Some(Action::Verify),
                    Held::ApproveKey => Some(Action::Approve),
                    _ => None,
                };
                if let Some(clicked) = self.pressed.take().filter(|a| Some(*a) == released) {
                    match clicked {
                        Action::Verify => self.verified = !self.verified,
                        Action::Approve if self.verified => action = Some(clicked),
                        Action::Cancel => action = Some(clicked),
                        _ => {}
                    }
                }
            }
            (previous, next) if previous != next => {
                // Mixing sources or switching directly between held controls
                // invalidates this entire gesture until all controls release.
                self.pressed = None;
            }
            _ => {}
        }
        self.held = held;
        action
    }
}

fn hit(x: usize, y: usize) -> Option<Action> {
    [
        (VERIFIED, Action::Verify),
        (APPROVE, Action::Approve),
        (CANCEL, Action::Cancel),
    ]
    .into_iter()
    .find(|(rect, _)| rect.contains(x, y))
    .map(|(_, action)| action)
}

fn label(canvas: &mut [u32], x: usize, y: usize, value: &str, scale: usize, color: u32) {
    text(
        canvas,
        (x, y),
        value,
        scale,
        (WIDTH - x) / (6 * scale),
        color,
    );
}

fn button(canvas: &mut [u32], rect: Rect, value: &str, color: u32) {
    overview::fill(canvas, rect, color);
    let count = value.len().min(rect.w / 12);
    text(
        canvas,
        (
            rect.x + (rect.w - count * 12) / 2,
            rect.y + (rect.h - 14) / 2,
        ),
        value,
        2,
        count,
        INK,
    );
}

/// All 253 possible DNS host bytes fit in four rows; IPv6 uses the same
/// untruncated representation. Full hexadecimal identities are shown.
fn render_review(canvas: &mut [u32], review: &Review<'_>, remaining: u64, verified: bool) {
    canvas.fill(BG);
    label(canvas, 48, 20, "INTRALINK / SECURE PAIRING", 2, ACCENT);
    label(canvas, 48, 49, review.stage.title(), 3, INK);
    label(canvas, 48, 80, review.stage.description(), 2, MUTED);
    label(
        canvas,
        48,
        101,
        "DIAGNOSTIC PAIRING ONLY / NO SCREEN OR INPUT SESSION",
        1,
        ACCENT,
    );
    label(
        canvas,
        48,
        122,
        "BASE / CONTROLLER IDENTITY - 64 HEX DIGITS",
        1,
        MUTED,
    );
    label(canvas, 48, 139, &review.controller.to_string(), 2, INK);
    label(
        canvas,
        48,
        171,
        "CLIENT / HOST IDENTITY - 64 HEX DIGITS",
        1,
        MUTED,
    );
    label(canvas, 48, 188, &review.host.to_string(), 2, INK);
    label(canvas, 48, 222, "CLIENT ROUTING HINT / FULL HOST", 1, MUTED);
    // Route validates ASCII and the maximum length before this view is used.
    for (index, row) in review.route.host().as_bytes().chunks(64).enumerate() {
        let value = std::str::from_utf8(row).expect("validated ASCII route");
        label(canvas, 48, 238 + index * 18, value, 2, INK);
    }
    label(
        canvas,
        48,
        315,
        &format!("UDP PORT {}", review.route.port()),
        2,
        INK,
    );
    let lifetime = match review.time {
        ReviewTime::New(ttl) => format!("INVITE LIFETIME {ttl} S / REVIEW {remaining} S LEFT"),
        ReviewTime::Expires(expires) => {
            format!("EXPIRES UTC {expires} / REVIEW {remaining} S LEFT")
        }
    };
    label(canvas, 48, 341, &lifetime, 2, MUTED);
    if let Some(name) = review.name {
        label(canvas, 48, 369, &format!("LOCAL LABEL {name}"), 2, MUTED);
    } else {
        label(canvas, 48, 369, "REPLACES ANY OUTSTANDING OFFER", 2, MUTED);
    }
    overview::fill(canvas, VERIFIED, SURFACE);
    overview::fill(
        canvas,
        Rect {
            x: 64,
            y: 411,
            w: 24,
            h: 24,
        },
        if verified { ACCENT } else { 0x425266 },
    );
    if verified {
        label(canvas, 70, 416, "X", 2, BG);
    }
    label(
        canvas,
        102,
        416,
        "I VERIFIED BOTH FULL IDENTITIES INDEPENDENTLY",
        2,
        INK,
    );
    label(
        canvas,
        48,
        452,
        "SPACE VERIFY / ENTER CONFIRM / ESC CANCEL",
        1,
        MUTED,
    );
    button(canvas, CANCEL, "CANCEL", SURFACE);
    button(
        canvas,
        APPROVE,
        review.stage.action(),
        if verified { 0x08685c } else { 0x35404e },
    );
}

fn window(title: &str) -> Result<Window> {
    let mut window = Window::new(title, WIDTH, HEIGHT, WindowOptions::default())
        .map_err(|error| anyhow::anyhow!("Pairing window: {error}"))?;
    window.set_target_fps(30);
    Ok(window)
}

fn position(window: &Window) -> Option<(usize, usize)> {
    window
        .get_mouse_pos(MouseMode::Discard)
        .and_then(|point| overview::map_point(window.get_size(), point))
}

fn present(window: &mut Window, canvas: &[u32]) -> Result<()> {
    window
        .update_with_buffer(canvas, WIDTH, HEIGHT)
        .map_err(|error| anyhow::anyhow!("Pairing window: {error}"))
}

pub(crate) fn check_available() -> Result<()> {
    super::require_graphical_terminal()
}

pub(crate) fn approve(review: &Review<'_>) -> Result<()> {
    check_available()?;
    let mut window = window("IntraLink | Review secure pairing")?;
    let mut canvas = vec![0; WIDTH * HEIGHT];
    let start = Instant::now();
    let mut input = ReviewInput::default();
    let mut painted = None;
    while window.is_open() && !window.is_key_down(Key::Escape) {
        let remaining = 300u64.saturating_sub(start.elapsed().as_secs());
        let remaining = match review.time {
            ReviewTime::New(_) => remaining,
            ReviewTime::Expires(expires) => remaining.min(expires.saturating_sub(now()?)),
        };
        ensure!(remaining > 0, "Pairing review expired; no change made");
        // First present the actual identities before accepting any clicks.
        let state = (remaining, input.verified);
        if painted != Some(state) {
            render_review(&mut canvas, review, remaining, input.verified);
            painted = Some(state);
        }
        present(&mut window, &canvas)?;
        if !window.is_open() || window.is_key_down(Key::Escape) {
            break;
        }
        let at = position(&window).and_then(|(x, y)| hit(x, y));
        match input.step(
            window.is_active(),
            window.get_mouse_down(MouseButton::Left),
            at,
            window.is_key_down(Key::Space),
            window.is_key_down(Key::Enter),
        ) {
            Some(Action::Approve) => return Ok(()),
            Some(Action::Cancel) => break,
            _ => {}
        }
    }
    anyhow::bail!("Pairing canceled; no change made")
}

/// An integer-scaled QR with the complete four-module white quiet zone.
/// No code is changed, truncated, or re-encoded as a short bearer token.
pub(crate) struct CodePanel {
    qr: QrCode,
}

impl CodePanel {
    pub(crate) fn new(value: &str) -> Result<Self> {
        ensure!(
            !value.is_empty() && value.len() <= intralink_vault::MAX_CODE_BYTES,
            "Invalid QR payload size"
        );
        Ok(Self {
            qr: QrCode::new(value.as_bytes())?,
        })
    }

    fn qr_rect(&self) -> (usize, usize, usize, usize) {
        let modules = self.qr.width() + 8;
        let scale = 440 / modules;
        let extent = modules * scale;
        (
            48 + (440 - extent) / 2,
            50 + (440 - extent) / 2,
            scale,
            extent,
        )
    }

    fn render(&self, canvas: &mut [u32], heading: &str, detail: &str, remaining: u64) {
        canvas.fill(BG);
        label(canvas, 48, 20, "INTRALINK / CONNECTION CODE", 2, ACCENT);
        let (x, y, scale, extent) = self.qr_rect();
        overview::fill(
            canvas,
            Rect {
                x,
                y,
                w: extent,
                h: extent,
            },
            0xffffff,
        );
        for row in 0..self.qr.width() {
            for col in 0..self.qr.width() {
                if self.qr[(col, row)] == Color::Dark {
                    overview::fill(
                        canvas,
                        Rect {
                            x: x + (col + 4) * scale,
                            y: y + (row + 4) * scale,
                            w: scale,
                            h: scale,
                        },
                        0x000000,
                    );
                }
            }
        }
        label(canvas, 518, 72, heading, 3, INK);
        label(canvas, 518, 120, detail, 2, ACCENT);
        for (i, line) in [
            "SCAN WITH A TRUSTED QR READER",
            "OR COPY THE EXACT TERMINAL CODE",
            "SEND TO THE INTENDED OWNER ONLY",
            "VERIFY THE FULL IDENTITIES",
            "THIS CODE IS NOT REMOTE ACCESS",
        ]
        .iter()
        .enumerate()
        {
            label(canvas, 518, 175 + i * 33, line, 2, MUTED);
        }
        label(
            canvas,
            518,
            363,
            &format!("DISPLAY CLOSES IN {remaining} S"),
            2,
            INK,
        );
        label(canvas, 518, 395, "ESC CLOSES THIS WINDOW", 1, MUTED);
        button(canvas, CLOSE, "CLOSE", SURFACE);
    }

    pub(crate) fn show(&self, heading: &str, detail: &str, expires: Option<u64>) -> Result<()> {
        check_available()?;
        let mut window = window("IntraLink | Connection code")?;
        let mut canvas = vec![0; WIDTH * HEIGHT];
        let start = Instant::now();
        let mut last = None;
        let mut click = ReviewInput::default();
        while window.is_open() && !window.is_key_down(Key::Escape) {
            let remaining = 300u64.saturating_sub(start.elapsed().as_secs());
            let remaining = match expires {
                Some(expires) => remaining.min(expires.saturating_sub(now()?)),
                None => remaining,
            };
            if remaining == 0 {
                break;
            }
            if last != Some(remaining) {
                self.render(&mut canvas, heading, detail, remaining);
                last = Some(remaining);
            }
            present(&mut window, &canvas)?;
            let at = position(&window)
                .filter(|&(x, y)| CLOSE.contains(x, y))
                .map(|_| Action::Cancel);
            if click.step(
                window.is_active(),
                window.get_mouse_down(MouseButton::Left),
                at,
                false,
                false,
            ) == Some(Action::Cancel)
            {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrollment::{ReviewStage, ReviewTime};
    use intralink_core::Role;
    use intralink_vault::{Invitation, Profile, Route};

    fn click(input: &mut ReviewInput, action: Action) -> Option<Action> {
        assert_eq!(input.step(true, true, Some(action), false, false), None);
        input.step(true, false, Some(action), false, false)
    }

    #[test]
    fn approval_requires_focused_neutral_complete_click_and_independent_verification() {
        let mut input = ReviewInput::default();
        assert_eq!(
            input.step(true, true, Some(Action::Approve), false, false),
            None
        );
        assert_eq!(
            input.step(true, false, Some(Action::Approve), false, false),
            None
        );
        assert_eq!(click(&mut input, Action::Approve), None);
        assert_eq!(click(&mut input, Action::Verify), None);
        assert!(input.verified);
        assert_eq!(
            input.step(true, true, Some(Action::Approve), false, false),
            None
        );
        assert_eq!(
            input.step(false, false, Some(Action::Approve), false, false),
            None
        );
        assert_eq!(
            input.step(true, false, Some(Action::Approve), false, false),
            None
        );
        assert_eq!(click(&mut input, Action::Approve), None);
        assert_eq!(click(&mut input, Action::Verify), None);
        assert_eq!(
            input.step(true, true, Some(Action::Approve), false, false),
            None
        );
        assert_eq!(
            input.step(true, false, Some(Action::Cancel), false, false),
            None
        );
        assert_eq!(click(&mut input, Action::Approve), Some(Action::Approve));
        assert_eq!(click(&mut input, Action::Cancel), Some(Action::Cancel));
    }

    #[test]
    fn keyboard_requires_two_distinct_released_actions_and_rejects_mixed_sources() {
        let mut input = ReviewInput::default();
        // Enter held at startup or before verification is never approval.
        assert_eq!(input.step(true, false, None, false, true), None);
        assert_eq!(input.step(true, false, None, false, false), None);
        assert_eq!(input.step(true, false, None, false, true), None);
        assert_eq!(input.step(true, false, None, false, false), None);
        assert_eq!(input.step(true, false, None, true, false), None);
        assert_eq!(input.step(true, false, None, true, true), None);
        assert_eq!(input.step(true, false, None, false, false), None);
        assert!(!input.verified);
        assert_eq!(input.step(true, false, None, true, false), None);
        assert_eq!(input.step(true, false, None, false, false), None);
        assert!(input.verified);
        assert_eq!(input.step(true, false, None, false, true), None);
        assert_eq!(
            input.step(true, true, Some(Action::Approve), false, true),
            None
        );
        assert_eq!(input.step(true, false, None, false, false), None);
        assert_eq!(input.step(true, false, None, false, true), None);
        assert_eq!(
            input.step(true, false, None, false, false),
            Some(Action::Approve)
        );
    }

    /// Developer-owned synthetic UI only. Never opens a vault or approves
    /// a session; skipped in ordinary/headless test runs.
    #[test]
    #[ignore = "requires a real local graphical desktop"]
    fn native_pairing_window_presents_generated_review_and_qr() -> Result<()> {
        super::super::require_graphical_environment()?;
        let mut host = Profile::new(Role::Host);
        let base = Profile::new(Role::Controller);
        let route = Route::new("192.168.10.2", 45100)?;
        let offer = host.create_invite(base.public_id(), route.clone(), 1000, 300)?;
        let review = Review {
            stage: ReviewStage::Issue,
            host: host.public_id(),
            controller: base.public_id(),
            route: &route,
            time: ReviewTime::New(300),
            name: None,
        };
        let mut canvas = vec![0; WIDTH * HEIGHT];
        let mut window = window("IntraLink | Generated pairing presentation test")?;
        render_review(&mut canvas, &review, 300, false);
        for _ in 0..8 {
            present(&mut window, &canvas)?;
            assert!(window.is_open());
        }
        CodePanel::new(&offer)?.render(&mut canvas, "Signed invite", "LOCAL TEST ONLY", 299);
        for _ in 0..8 {
            present(&mut window, &canvas)?;
            assert!(window.is_open());
        }
        println!("PAIRING_NATIVE_PRESENT_PASS review=8 qr=8; generated identities only");
        Ok(())
    }

    #[test]
    fn full_signed_offer_and_acceptance_render_without_quiet_zone_loss() -> Result<()> {
        let mut host = Profile::new(Role::Host);
        let mut base = Profile::new(Role::Controller);
        let host_name = [
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61),
        ]
        .join(".");
        assert_eq!(host_name.len(), 253);
        let route = Route::new(&host_name, 45100)?;
        let offer = host.create_invite(base.public_id(), route.clone(), 1000, 300)?;
        let invite = Invitation::inspect_offer(&offer, 1001)?;
        let review = Review {
            stage: ReviewStage::Accept,
            host: invite.host()?,
            controller: invite.controller()?,
            route: &route,
            time: ReviewTime::Expires(invite.expires()),
            name: Some("client"),
        };
        let mut canvas = vec![0; WIDTH * HEIGHT];
        render_review(&mut canvas, &review, 299, false);
        assert!(canvas[238 * WIDTH..310 * WIDTH].contains(&INK));
        assert_eq!(canvas[APPROVE.y * WIDTH + APPROVE.x], 0x35404e);
        render_review(&mut canvas, &review, 299, true);
        assert_eq!(canvas[APPROVE.y * WIDTH + APPROVE.x], 0x08685c);
        let acceptance = base.accept_invite("client", &offer, 1001)?;
        for code in [&offer, &acceptance] {
            let panel = CodePanel::new(code)?;
            panel.render(&mut canvas, "SIGNED CODE", "NO SESSION STARTED", 299);
            let (x, y, scale, extent) = panel.qr_rect();
            assert!(scale >= 2 && x + extent <= 488 && y + extent <= 490);
            for row in 0..extent {
                for col in 0..extent {
                    let m = (col / scale, row / scale);
                    let expected = if m.0 < 4
                        || m.1 < 4
                        || m.0 >= panel.qr.width() + 4
                        || m.1 >= panel.qr.width() + 4
                    {
                        0xffffff
                    } else if panel.qr[(m.0 - 4, m.1 - 4)] == Color::Dark {
                        0
                    } else {
                        0xffffff
                    };
                    assert_eq!(canvas[(y + row) * WIDTH + x + col], expected);
                }
            }
        }
        assert!(Invitation::inspect_acceptance(&offer, 1001).is_err());
        assert!(Invitation::inspect_offer(&offer, 1300).is_err());
        host.confirm_invite("base", &acceptance, 1002)?;
        assert_eq!(host.peers().len(), 1);
        assert_eq!(base.peers().len(), 1);
        assert!(host.confirm_invite("base2", &acceptance, 1003).is_err());
        Ok(())
    }
}
