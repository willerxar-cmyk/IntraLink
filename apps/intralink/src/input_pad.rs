//! Named spatial input cards and local-only controls. The grid geometry and
//! normalized coordinates remain owned by intralink-core.
use crate::desktop::draw_label;
use intralink_core::{
    input_layout::{Layout, Zone},
    overview::{self, Rect},
};
use minifb::{MouseMode, Window};

pub const WIDTH: usize = 960;
pub const HEIGHT: usize = 540;
const BACKGROUND: u32 = 0x101923;
const SURFACE: u32 = 0x223245;
const LOCAL: u32 = 0x314253;
const ACTIVE: u32 = 0x176452;
const READY: u32 = 0x63ddc2;
const MUTED: u32 = 0xa8b9ca;
const FOREGROUND: u32 = 0xeff5fa;
const OFFLINE: u32 = 0x35404e;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalControl {
    Arm,
    Portal,
    Reset,
}

pub fn pointer(window: &Window) -> Option<(usize, usize)> {
    let (x, y) = window.get_mouse_pos(MouseMode::Discard)?;
    let (w, h) = window.get_size();
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
        return None;
    }
    Some((x as usize, y as usize))
}

pub fn pointer_zone(window: &Window, layout: Layout) -> Option<Zone> {
    let (x, y) = pointer(window)?;
    let (width, height) = window.get_size();
    layout.zone(x, y, width, height)
}

fn controls(layout: Layout) -> Option<[(LocalControl, Rect); 3]> {
    let local = layout.cell_of(None)?;
    let (l, t, r, b) = layout.bounds(local, WIDTH, HEIGHT)?;
    let width = (r - l - 40) / 3;
    let y = b - 51;
    if y < t + 100 {
        return None;
    }
    Some([
        (
            LocalControl::Arm,
            Rect {
                x: l + 12,
                y,
                w: width,
                h: 37,
            },
        ),
        (
            LocalControl::Portal,
            Rect {
                x: l + 20 + width,
                y,
                w: width,
                h: 37,
            },
        ),
        (
            LocalControl::Reset,
            Rect {
                x: l + 28 + width * 2,
                y,
                w: width,
                h: 37,
            },
        ),
    ])
}

pub fn local_control(layout: Layout, point: (usize, usize)) -> Option<LocalControl> {
    controls(layout)?
        .into_iter()
        .find(|(_, rect)| rect.contains(point.0, point.1))
        .map(|(control, _)| control)
}

fn fill(buffer: &mut [u32], x: usize, y: usize, w: usize, h: usize, color: u32) {
    let _ = overview::fill(buffer, Rect { x, y, w, h }, color);
}

fn monitor(buffer: &mut [u32], x: usize, y: usize, tint: u32, panel: u32) {
    fill(buffer, x, y, 72, 46, tint);
    fill(buffer, x + 3, y + 3, 66, 38, panel);
    fill(buffer, x + 32, y + 46, 8, 11, tint);
    fill(buffer, x + 23, y + 57, 26, 3, tint);
}

fn button(buffer: &mut [u32], rect: Rect, label: &str, color: u32) {
    let _ = overview::fill(buffer, rect, color);
    let n = label.chars().count().min(rect.w / 12);
    draw_label(
        buffer,
        (rect.x + (rect.w - n * 12) / 2, rect.y + 11),
        label,
        2,
        n,
        FOREGROUND,
    );
}

pub struct View<'a> {
    pub names: &'a [&'a str],
    pub ready: u8,
    pub active: Option<usize>,
    pub armed: bool,
    pub portal: &'a str,
    pub layout_status: &'a str,
}

pub fn render(layout: Layout, buffer: &mut [u32], view: View<'_>) {
    assert_eq!(buffer.len(), WIDTH * HEIGHT);
    buffer.fill(BACKGROUND);
    for cell in 0..layout.cells() {
        let Some(label) = layout.label(cell) else {
            continue;
        };
        let (l, t, r, b) = layout
            .bounds(cell, WIDTH, HEIGHT)
            .expect("fixed valid pad size");
        let target = label.checked_sub(1);
        let color = match target {
            None => LOCAL,
            Some(slot) if view.active == Some(slot) => ACTIVE,
            Some(slot) if view.ready & (1 << slot) == 0 => OFFLINE,
            Some(_) => SURFACE,
        };
        fill(buffer, l + 5, t + 5, r - l - 10, b - t - 10, color);
        let (name, status, tint) = match target {
            None => (
                "THIS DEVICE",
                if view.armed {
                    "BASE / ARMED"
                } else {
                    "BASE / SAFE"
                },
                READY,
            ),
            Some(slot) => (
                view.names.get(slot).copied().unwrap_or("CLIENT"),
                if view.active == Some(slot) {
                    "ACTIVE INPUT"
                } else if view.ready & (1 << slot) != 0 {
                    "READY"
                } else {
                    "NOT CONNECTED"
                },
                if view.active == Some(slot) || view.ready & (1 << slot) != 0 {
                    READY
                } else {
                    MUTED
                },
            ),
        };
        fill(buffer, l + 5, t + 5, 3, b - t - 10, tint);
        draw_label(
            buffer,
            (l + 16, t + 15),
            name,
            2,
            (r - l - 32) / 12,
            FOREGROUND,
        );
        draw_label(buffer, (l + 16, t + 36), status, 1, (r - l - 32) / 6, tint);
        if target.is_none() {
            draw_label(
                buffer,
                (l + 16, t + 65),
                if view.armed {
                    "F12 STOPS INPUT"
                } else {
                    "DRAG CARDS TO ARRANGE"
                },
                2,
                (r - l - 32) / 12,
                FOREGROUND,
            );
            draw_label(
                buffer,
                (l + 16, t + 89),
                if view.armed {
                    "MOVE TO A READY SCREEN"
                } else {
                    view.layout_status
                },
                1,
                (r - l - 32) / 6,
                MUTED,
            );
        } else {
            monitor(buffer, (l + r) / 2 - 36, t + 61, tint, color);
            let label = format!("CLIENT {label}");
            draw_label(
                buffer,
                (l + 16, b - 28),
                &label,
                2,
                (r - l - 32) / 12,
                MUTED,
            );
        }
    }
    if let Some(controls) = controls(layout) {
        for (control, rect) in controls {
            match control {
                LocalControl::Arm => button(
                    buffer,
                    rect,
                    if view.armed { "STOP" } else { "START" },
                    if view.armed { 0x6e3443 } else { 0x08685c },
                ),
                LocalControl::Portal => button(
                    buffer,
                    rect,
                    view.portal,
                    if view.armed { LOCAL } else { SURFACE },
                ),
                LocalControl::Reset => button(
                    buffer,
                    rect,
                    "RESET",
                    if view.armed { LOCAL } else { SURFACE },
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_card_renders_distinct_state_and_controls_follow_local() {
        let mut layout = Layout::arranged(8).unwrap();
        let names = [
            "workstation",
            "tablet",
            "laptop",
            "pc",
            "client5",
            "client6",
            "client7",
            "client8",
        ];
        let mut pixels = vec![0; WIDTH * HEIGHT];
        render(
            layout,
            &mut pixels,
            View {
                names: &names,
                ready: 255,
                active: Some(7),
                armed: true,
                portal: "FULL",
                layout_status: "LAYOUT RESTORED",
            },
        );
        let (l, t, _, _) = layout
            .bounds(layout.cell_of(Some(7)).unwrap(), WIDTH, HEIGHT)
            .unwrap();
        assert_eq!(pixels[(t + 10) * WIDTH + l + 10], ACTIVE);
        let old = controls(layout).unwrap()[0].1;
        assert_eq!(
            local_control(layout, (old.x, old.y)),
            Some(LocalControl::Arm)
        );
        assert!(layout.relocate(4, 8));
        assert_eq!(local_control(layout, (old.x, old.y)), None);
        for (control, rect) in controls(layout).unwrap() {
            assert_eq!(local_control(layout, (rect.x, rect.y)), Some(control));
            assert_eq!(
                layout.zone(rect.x, rect.y, WIDTH, HEIGHT).unwrap().target,
                None
            );
        }
    }

    #[test]
    fn offline_and_ready_cards_never_share_local_controls() {
        let layout = Layout::arranged(2).unwrap();
        let mut pixels = vec![0; WIDTH * HEIGHT];
        render(
            layout,
            &mut pixels,
            View {
                names: &["ONE", "TWO"],
                ready: 1,
                active: None,
                armed: false,
                portal: "OFF",
                layout_status: "NEW LAYOUT",
            },
        );
        let (l, t, _, _) = layout
            .bounds(layout.cell_of(Some(1)).unwrap(), WIDTH, HEIGHT)
            .unwrap();
        assert_eq!(pixels[(t + 10) * WIDTH + l + 10], OFFLINE);
        assert_eq!(local_control(layout, (l + 20, t + 130)), None);
        assert_eq!(
            layout.zone(l + 20, t + 130, WIDTH, HEIGHT).unwrap().target,
            Some(1)
        );
    }
}
