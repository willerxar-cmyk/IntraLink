//! Local-only first-run role chooser. Never opens a listener or captures input
//! outside its own focused window. Credentials are entered in the terminal.
use super::overview::text;
use anyhow::{Result, bail};
use intralink_core::{
    Role,
    overview::{self, HEIGHT, Rect, WIDTH},
};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};

const BASE: Rect = Rect {
    x: 48,
    y: 170,
    w: 420,
    h: 244,
};
const CLIENT: Rect = Rect {
    x: 492,
    y: 170,
    w: 420,
    h: 244,
};
const BACKGROUND: u32 = 0x111b2b;
const FOREGROUND: u32 = 0xf2f7fb;
const MUTED: u32 = 0xaabbd0;

fn hit(x: usize, y: usize) -> Option<Role> {
    if BASE.contains(x, y) {
        Some(Role::Controller)
    } else if CLIENT.contains(x, y) {
        Some(Role::Host)
    } else {
        None
    }
}

fn render(canvas: &mut [u32], hover: Option<Role>) {
    canvas.fill(BACKGROUND);
    text(canvas, (48, 42), "INTRALINK / FIRST RUN", 2, 42, 0x68d8bf);
    text(
        canvas,
        (48, 91),
        "HOW WILL YOU USE THIS DEVICE?",
        3,
        42,
        FOREGROUND,
    );
    for (rect, role, title, subtitle, detail, key) in [
        (
            BASE,
            Role::Controller,
            "BASE",
            "CONTROL OTHER DEVICES",
            "KEYBOARD AND MOUSE ORIGIN",
            "PRESS 1",
        ),
        (
            CLIENT,
            Role::Host,
            "CLIENT",
            "RECEIVE CONTROL",
            "AUTHORIZE A BASE LOCALLY",
            "PRESS 2",
        ),
    ] {
        let selected = hover == Some(role);
        overview::fill(canvas, rect, if selected { 0x205b66 } else { 0x25374b });
        overview::fill(
            canvas,
            Rect {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: 5,
            },
            if selected { 0x68d8bf } else { 0x527087 },
        );
        text(canvas, (rect.x + 25, rect.y + 30), title, 5, 13, FOREGROUND);
        text(
            canvas,
            (rect.x + 25, rect.y + 101),
            subtitle,
            2,
            30,
            FOREGROUND,
        );
        text(canvas, (rect.x + 25, rect.y + 139), detail, 2, 30, MUTED);
        text(canvas, (rect.x + 25, rect.y + 201), key, 2, 30, 0x68d8bf);
    }
    text(
        canvas,
        (48, 450),
        "CLICK A ROLE OR USE 1 / 2. ESC TO CANCEL.",
        2,
        68,
        MUTED,
    );
    text(
        canvas,
        (48, 483),
        "NEXT: NAME AND PASSPHRASE IN THE LOCAL TERMINAL.",
        2,
        72,
        MUTED,
    );
}

pub(crate) fn choose_setup_role() -> Result<Role> {
    super::require_graphical_terminal()?;
    let mut window = Window::new(
        "IntraLink | Choose BASE or CLIENT",
        WIDTH,
        HEIGHT,
        WindowOptions::default(),
    )
    .map_err(|error| anyhow::anyhow!("Role chooser: {error}"))?;
    window.set_target_fps(30);
    let mut canvas = vec![0; WIDTH * HEIGHT];
    // A click must begin and end in the same card while focused. Opening
    // the chooser under an already held mouse button cannot select a role.
    let mut armed = false;
    let mut previous_down = true;
    let mut pressed = None;
    while window.is_open() && !window.is_key_down(Key::Escape) {
        let hover = window
            .get_mouse_pos(MouseMode::Discard)
            .and_then(|point| overview::map_point(window.get_size(), point))
            .and_then(|(x, y)| hit(x, y));
        let down = window.get_mouse_down(MouseButton::Left);
        if window.is_active() {
            if !down && !armed {
                armed = true;
            }
            if armed && down && !previous_down {
                pressed = hover;
            }
            if !down
                && previous_down
                && let Some(role) = pressed.take()
                && hover == Some(role)
            {
                return Ok(role);
            }
            if window.is_key_pressed(Key::Key1, KeyRepeat::No) {
                return Ok(Role::Controller);
            }
            if window.is_key_pressed(Key::Key2, KeyRepeat::No) {
                return Ok(Role::Host);
            }
        } else {
            armed = false;
            pressed = None;
        }
        previous_down = down;
        render(&mut canvas, hover);
        window
            .update_with_buffer(&canvas, WIDTH, HEIGHT)
            .map_err(|error| anyhow::anyhow!("Role chooser: {error}"))?;
    }
    bail!("Setup canceled; no identity was created")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cards_are_disjoint_and_render_with_selection() {
        assert_eq!(hit(48, 170), Some(Role::Controller));
        assert_eq!(hit(492, 170), Some(Role::Host));
        assert_eq!(hit(480, 200), None);
        assert_eq!(hit(48, 414), None);
        let mut canvas = vec![0; WIDTH * HEIGHT];
        render(&mut canvas, None);
        let initial = canvas[(BASE.y + 10) * WIDTH + BASE.x + 10];
        render(&mut canvas, Some(Role::Controller));
        assert_ne!(initial, canvas[(BASE.y + 10) * WIDTH + BASE.x + 10]);
        assert_eq!(canvas[0], BACKGROUND);
    }
}
