//! Common proportional canvas for the offline editor and focused geometry pad.
use crate::desktop::draw_label;
use intralink_core::{
    display_geometry::{Placement, Viewport},
    overview::{self, HEIGHT, Rect, WIDTH},
    screen_topology::{Screen, Topology},
};

pub(super) const BG: u32 = 0x101923;
pub(super) const CARD: u32 = 0x223245;
pub(super) const INK: u32 = 0xeff5fa;
pub(super) const MUTED: u32 = 0xa8b9ca;
pub(super) const ACCENT: u32 = 0x63ddc2;
pub(super) const DISABLED: u32 = 0x35404e;

pub(super) const EDIT_AREA: Rect = Rect {
    x: 16,
    y: 64,
    w: 600,
    h: 390,
};
pub(super) const LIVE_AREA: Rect = Rect {
    x: 16,
    y: 50,
    w: 928,
    h: 430,
};

#[derive(Clone, Copy)]
pub(super) struct Projection {
    view: Viewport,
    area: Rect,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Sample {
    pub id: u8,
    pub point: (i32, i32),
    pub normalized: (u16, u16),
}

impl Projection {
    pub fn new(topology: &Topology, area: Rect) -> Option<Self> {
        Some(Self {
            view: Viewport::fit(topology, area.w.try_into().ok()?, area.h.try_into().ok()?)?,
            area,
        })
    }
    pub fn rect(self, screen: Screen) -> Rect {
        let (l, t, r, b) = self.view.rect(screen);
        Rect {
            x: self.area.x + l as usize,
            y: self.area.y + t as usize,
            w: (r - l) as usize,
            h: (b - t) as usize,
        }
    }
    pub fn sample(self, topology: &Topology, at: (usize, usize)) -> Option<Sample> {
        if !self.area.contains(at.0, at.1) {
            return None;
        }
        let point = self
            .view
            .point((at.0 - self.area.x) as i32, (at.1 - self.area.y) as i32)?;
        let id = topology.at(point.0, point.1)?;
        let screen = *topology.screens().iter().find(|s| s.id == id)?;
        let rect = self.rect(screen);
        if !rect.contains(at.0, at.1) {
            return None;
        }
        // Map each visible screen's first and last pad pixel to its actual
        // logical endpoints; never truncate the right/bottom remote edge.
        let axis = |position: usize, origin: usize, extent: usize, logical: u16| {
            let pixel = (position - origin) as u64;
            let denominator = (extent - 1) as u64;
            ((pixel * u64::from(logical - 1) + denominator / 2) / denominator) as i32
        };
        let point = (
            screen.x + axis(at.0, rect.x, rect.w, screen.width),
            screen.y + axis(at.1, rect.y, rect.h, screen.height),
        );
        Some(Sample {
            id,
            point,
            normalized: topology.normalized(id, point.0, point.1)?,
        })
    }
    pub fn delta(self, x: i32, y: i32) -> (i32, i32) {
        self.view.delta(x, y)
    }
}

pub(super) fn label(canvas: &mut [u32], x: usize, y: usize, value: &str, scale: usize, color: u32) {
    if x >= WIDTH || y >= HEIGHT {
        return;
    }
    draw_label(
        canvas,
        (x, y),
        value,
        scale,
        (WIDTH - x) / (6 * scale),
        color,
    );
}
pub(super) fn button(canvas: &mut [u32], r: Rect, value: &str, enabled: bool) {
    overview::fill(canvas, r, if enabled { 0x08685c } else { DISABLED });
    let n = value.len().min(r.w / 12);
    draw_label(
        canvas,
        (r.x + (r.w - n * 12) / 2, r.y + (r.h - 14) / 2),
        value,
        2,
        n,
        INK,
    );
}

pub(super) struct Cards<'a> {
    pub names: &'a [&'a str],
    pub placements: &'a [Placement],
    pub confirmed: u16,
    pub selected: Option<u8>,
    pub ready: u8,
    pub active: Option<usize>,
    pub editing: bool,
}

pub(super) fn render_cards(
    canvas: &mut [u32],
    topology: &Topology,
    projection: Projection,
    view: Cards<'_>,
) {
    for screen in topology.screens().iter().copied() {
        let rect = projection.rect(screen);
        let i = usize::from(screen.id);
        let p = view.placements[i];
        let confirmed = view.confirmed & (1 << i) != 0;
        let selected = view.selected == Some(screen.id);
        let active = i > 0 && view.active == Some(i - 1);
        let ready = i > 0 && view.ready & (1 << (i - 1)) != 0;
        let border = if selected || active || i == 0 {
            ACCENT
        } else {
            MUTED
        };
        let color = if active {
            0x176452
        } else if selected {
            0x174c40
        } else if view.editing || ready || i == 0 {
            CARD
        } else {
            DISABLED
        };
        overview::fill(canvas, rect, border);
        if rect.w > 2 && rect.h > 2 {
            overview::fill(
                canvas,
                Rect {
                    x: rect.x + 1,
                    y: rect.y + 1,
                    w: rect.w - 2,
                    h: rect.h - 2,
                },
                color,
            );
        }
        if rect.w < 20 || rect.h < 12 {
            continue;
        }
        let name = view.names.get(i).copied().unwrap_or("CLIENT");
        let max = (rect.w - 8) / 6;
        draw_label(canvas, (rect.x + 4, rect.y + 4), name, 1, max, INK);
        if rect.h >= 26 {
            let info = if !confirmed {
                "UNSET".to_owned()
            } else if view.editing || i == 0 {
                format!(
                    "{}X{} / {}%",
                    p.display.width, p.display.height, p.display.scale
                )
            } else if active {
                "ACTIVE INPUT".into()
            } else if ready {
                "READY".into()
            } else {
                "NOT CONNECTED".into()
            };
            draw_label(
                canvas,
                (rect.x + 4, rect.y + 15),
                &info,
                1,
                max,
                if confirmed { MUTED } else { 0xffbd70 },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use intralink_core::display_geometry::{Display, topology};
    #[test]
    fn displayed_screen_endpoints_cover_the_entire_normalized_range() {
        let display = Display {
            width: 3840,
            height: 2160,
            scale: 150,
            rotation: 0,
        };
        let p = [
            Placement {
                id: 0,
                x: 0,
                y: 0,
                display,
            },
            Placement {
                id: 1,
                x: 2560,
                y: 100,
                display: Display {
                    width: 1080,
                    height: 1920,
                    scale: 125,
                    rotation: 90,
                },
            },
        ];
        let t = topology(&p).unwrap();
        let view = Projection::new(&t, LIVE_AREA).unwrap();
        for s in t.screens() {
            let r = view.rect(*s);
            let first = view.sample(&t, (r.x, r.y)).unwrap();
            let last = view.sample(&t, (r.x + r.w - 1, r.y + r.h - 1)).unwrap();
            assert_eq!(first.id, s.id);
            assert_eq!(last.id, s.id);
            assert_eq!(first.normalized, (0, 0));
            assert_eq!(last.normalized, (65535, 65535));
        }
        assert!(view.sample(&t, (0, 0)).is_none());
    }
}
