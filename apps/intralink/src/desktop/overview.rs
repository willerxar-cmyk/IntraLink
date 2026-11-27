//! Local-only device overview. Render thumbnails at a bounded cadence, not at the media rate.
use intralink_core::{
    overview::{self, HEIGHT, Layout, Rect, WIDTH},
    presentation::Snapshot,
    telemetry::Rates,
    workspace::MAX_DEVICES,
};

const BACKGROUND: u32 = 0x101923;
const CARD: u32 = 0x243344;
const SELECTED: u32 = 0x157e72;
const BUTTON: u32 = 0x304256;
const FOREGROUND: u32 = 0xeaf0f5;
const MUTED: u32 = 0xaabccc;
const ACTIVE: u32 = 0x176c55;
const INACTIVE: u32 = 0x4a5563;

pub(super) struct Overview {
    pub layout: Layout,
    pixels: Vec<u32>,
    sequences: [usize; MAX_DEVICES],
}

impl Overview {
    pub fn new(count: usize) -> Option<Self> {
        Some(Self {
            layout: Layout::new(count)?,
            pixels: Vec::new(),
            sequences: [0; MAX_DEVICES],
        })
    }

    pub fn clear(&mut self) {
        self.pixels.fill(BACKGROUND);
        self.sequences.fill(0);
    }

    pub fn sequence(&self, index: usize) -> usize {
        self.sequences.get(index).copied().unwrap_or(0)
    }

    // Forget the previous session before a slot can be reused.
    pub fn invalidate(&mut self, index: usize) {
        if let Some(sequence) = self.sequences.get_mut(index) {
            *sequence = 0;
        }
        if let Some(card) = self.layout.card(index) {
            let _ = overview::fill(&mut self.pixels, card.preview, 0x111c28);
        }
    }
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    pub fn begin(&mut self, all_live: bool, fullscreen: bool) {
        self.pixels.resize(WIDTH * HEIGHT, BACKGROUND);
        let _ = overview::fill(
            &mut self.pixels,
            Rect {
                x: 0,
                y: 0,
                w: WIDTH,
                h: 52,
            },
            BACKGROUND,
        );
        text(
            &mut self.pixels,
            (12, 11),
            "INTRALINK / DEVICES",
            3,
            30,
            FOREGROUND,
        );
        text(
            &mut self.pixels,
            (12, 36),
            "SELECT A SCREEN OR TOGGLE ITS SESSION",
            1,
            60,
            MUTED,
        );
        button(
            &mut self.pixels,
            overview::MODE,
            if all_live { "ALL LIVE" } else { "ADAPTIVE" },
            if all_live { ACTIVE } else { BUTTON },
        );
        button(
            &mut self.pixels,
            overview::FULLSCREEN,
            if fullscreen { "WINDOWED" } else { "FULLSCREEN" },
            BUTTON,
        );
        button(&mut self.pixels, overview::BACK, "BACK", BUTTON);
    }

    pub fn card(
        &mut self,
        index: usize,
        name: &str,
        selected: bool,
        state: CardState<'_>,
        rates: Option<Rates>,
    ) {
        self.card_with_status(index, name, selected, state, rates, false);
    }

    pub fn card_with_status(
        &mut self,
        index: usize,
        name: &str,
        selected: bool,
        state: CardState<'_>,
        rates: Option<Rates>,
        no_update: bool,
    ) {
        let Some(card) = self.layout.card(index) else {
            return;
        };
        let outer = card.outer;
        let preview = card.preview;
        let background = if selected { SELECTED } else { CARD };
        // Draw only the chrome around the thumbnail. When the decoder owns
        // the surface lock, preserve the previous complete thumbnail in-place.
        for rect in [
            Rect {
                x: outer.x,
                y: outer.y,
                w: outer.w,
                h: preview.y - outer.y,
            },
            Rect {
                x: outer.x,
                y: preview.y,
                w: preview.x - outer.x,
                h: preview.h,
            },
            Rect {
                x: preview.x + preview.w,
                y: preview.y,
                w: outer.x + outer.w - preview.x - preview.w,
                h: preview.h,
            },
            Rect {
                x: outer.x,
                y: preview.y + preview.h,
                w: outer.w,
                h: outer.y + outer.h - preview.y - preview.h,
            },
        ] {
            let _ = overview::fill(&mut self.pixels, rect, background);
        }
        let available = card.toggle.x.saturating_sub(outer.x + 12) / 12;
        let label = format!("{} {}", index + 1, name);
        text(
            &mut self.pixels,
            (outer.x + 8, outer.y + 10),
            &label,
            2,
            available,
            FOREGROUND,
        );
        let (toggle, color, message) = match &state {
            CardState::Inactive => ("OFF", INACTIVE, "DISCONNECTED"),
            CardState::Closing => ("OFF", INACTIVE, "CLOSING"),
            CardState::Waiting | CardState::Retain => ("ON", ACTIVE, "CONNECTING / DECODING"),
            CardState::Live(_) => ("ON", ACTIVE, ""),
        };
        button(&mut self.pixels, card.toggle, toggle, color);
        match state {
            CardState::Live(snapshot) => {
                if snapshot.sequence() != self.sequence(index) {
                    if overview::thumbnail(
                        &mut self.pixels,
                        preview,
                        snapshot.pixels(),
                        snapshot.dimensions(),
                    ) {
                        self.sequences[index] = snapshot.sequence();
                    } else {
                        self.invalidate(index);
                    }
                }
            }
            CardState::Retain if self.sequence(index) > 0 => {}
            _ => {
                self.invalidate(index);
                let max = preview.w.saturating_sub(16) / 12;
                text(
                    &mut self.pixels,
                    (preview.x + 8, preview.y + 12),
                    message,
                    2,
                    max,
                    MUTED,
                );
            }
        }
        let label = if toggle == "OFF" {
            "SESSION INACTIVE".to_owned()
        } else if no_update {
            "LAST FRAME / NO NEW SAMPLE".to_owned()
        } else {
            rates.map_or_else(
                || "RX / DEC / VIEW - MEASURING".to_owned(),
                |rate| {
                    format!(
                        "RX {:.0} DEC {:.0} VIEW {:.0} / {:.1} MBPS",
                        rate.received_fps, rate.decoded_fps, rate.presented_fps, rate.payload_mbps
                    )
                },
            )
        };
        let max = outer.w.saturating_sub(16) / 6;
        text(
            &mut self.pixels,
            (outer.x + 8, preview.y + preview.h + 7),
            &label,
            1,
            max,
            FOREGROUND,
        );
    }
}

pub(super) enum CardState<'a> {
    Inactive,
    Closing,
    Waiting,
    Retain,
    Live(&'a Snapshot),
}

fn button(canvas: &mut [u32], rect: Rect, label: &str, color: u32) {
    let _ = overview::fill(canvas, rect, color);
    let chars = label.chars().count().min(rect.w / 12);
    let x = rect.x + (rect.w - chars * 12) / 2;
    let y = rect.y + rect.h.saturating_sub(14) / 2;
    text(canvas, (x, y), label, 2, chars, FOREGROUND);
}

/// Small built-in bitmap labels for the experimental viewer. ASCII is rendered
/// locally; unsupported characters show '?' and the full name remains in the title.
pub(crate) fn text(
    canvas: &mut [u32],
    origin: (usize, usize),
    value: &str,
    scale: usize,
    max: usize,
    color: u32,
) {
    for (i, character) in value.chars().take(max).enumerate() {
        let x = origin.0 + i * 6 * scale;
        for (row, mask) in glyph(character.to_ascii_uppercase()).iter().enumerate() {
            for col in 0..5 {
                if mask & (1 << (4 - col)) != 0 {
                    let _ = overview::fill(
                        canvas,
                        Rect {
                            x: x + col * scale,
                            y: origin.1 + row * scale,
                            w: scale,
                            h: scale,
                        },
                        color,
                    );
                }
            }
        }
    }
}

/// Reuse one viewer-owned scratch canvas only while a notice is displayed.
/// Do not modify the ordered decoder surface or count a notice as a frame.
pub(super) fn last_frame_banner(
    pixels: &[u32],
    dimensions: (usize, usize),
    output: &mut Vec<u32>,
) -> std::io::Result<()> {
    let (width, height) = dimensions;
    if width < 320 || height < 40 || width.checked_mul(height) != Some(pixels.len()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid notice canvas",
        ));
    }
    output.clear();
    output.extend_from_slice(pixels);
    let scale = if width >= 1280 { 3 } else { 2 };
    let band = 7 * scale + 16;
    output[..width * band].fill(0x593a0e);
    let message = "LAST FRAME / NO NEW NATIVE SAMPLE";
    for (i, c) in message.chars().take((width - 16) / (6 * scale)).enumerate() {
        let x = 8 + i * 6 * scale;
        for (row, mask) in glyph(c).iter().enumerate() {
            for col in 0..5 {
                if mask & (1 << (4 - col)) != 0 {
                    for dy in 0..scale {
                        let offset = (8 + row * scale + dy) * width + x + col * scale;
                        output[offset..offset + scale].fill(FOREGROUND);
                    }
                }
            }
        }
    }
    Ok(())
}

pub(super) fn glyph(c: char) -> [u8; 7] {
    match c {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 2, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ' ' => [0; 7],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        _ => [14, 17, 1, 2, 4, 0, 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use intralink_core::presentation::{PresentationSurface, SurfaceError};

    #[test]
    fn local_notice_preserves_original_pixels_and_reuses_scratch() {
        let pixels = vec![0x123456; 960 * 540];
        let mut scratch = Vec::new();
        last_frame_banner(&pixels, (960, 540), &mut scratch).unwrap();
        assert!(scratch[..960 * 30].contains(&FOREGROUND));
        assert_eq!(&scratch[960 * 30..], &pixels[960 * 30..]);
        assert!(pixels.iter().all(|p| *p == 0x123456));
        let ptr = scratch.as_ptr();
        last_frame_banner(&pixels, (960, 540), &mut scratch).unwrap();
        assert_eq!(ptr, scratch.as_ptr());
        assert!(last_frame_banner(&pixels, (1, 1), &mut scratch).is_err());
    }

    #[test]
    fn disconnected_card_removes_all_previous_pixels() {
        const SAMPLE: u32 = 0xff00ff;
        let mut panel = Overview::new(3).unwrap();
        assert!(panel.pixels().is_empty());
        panel.begin(false, true);
        let mut surface = PresentationSurface::default();
        surface
            .update::<SurfaceError>(1, (2, 2), |pixels| {
                pixels.resize(4, SAMPLE);
                Ok(())
            })
            .unwrap();
        let snapshot = surface.snapshot_after(0).unwrap();
        panel.card(0, "host-1", true, CardState::Live(&snapshot), None);
        assert!(panel.pixels().contains(&SAMPLE));
        panel.card(1, "host-2", false, CardState::Waiting, None);
        panel.card(2, "host-3", false, CardState::Inactive, None);
        panel.card(0, "host-1", true, CardState::Closing, None);
        assert!(!panel.pixels().contains(&SAMPLE));
        panel.card(0, "host-1", true, CardState::Inactive, None);
        assert!(!panel.pixels().contains(&SAMPLE));
        panel.clear();
        assert!(panel.pixels().iter().all(|pixel| *pixel == BACKGROUND));
    }

    #[test]
    fn all_supported_card_counts_render_in_one_reusable_canvas() {
        assert!(Overview::new(0).is_none());
        assert!(Overview::new(9).is_none());
        for count in 1..=8 {
            let mut panel = Overview::new(count).unwrap();
            panel.begin(true, false);
            let before = panel.pixels().as_ptr();
            for index in 0..count {
                panel.card(
                    index,
                    "Very-long-device-name-12345-ó",
                    index == 0,
                    CardState::Waiting,
                    None,
                );
            }
            assert_eq!(panel.pixels().len(), WIDTH * HEIGHT);
            panel.begin(false, true);
            assert_eq!(panel.pixels().as_ptr(), before);
            assert!(!panel.pixels().iter().all(|pixel| *pixel == BACKGROUND));
        }
    }

    #[test]
    fn busy_decoder_retains_thumbnail_but_session_restart_invalidates_it() {
        const FIRST: u32 = 0xff00ff;
        const NEXT: u32 = 0x00ffff;
        let mut panel = Overview::new(1).unwrap();
        let mut surface = PresentationSurface::default();
        surface
            .update::<SurfaceError>(1, (2, 2), |pixels| {
                pixels.resize(4, FIRST);
                Ok(())
            })
            .unwrap();
        panel.begin(false, true);
        let snapshot = surface.snapshot_after(0).unwrap();
        panel.card(0, "host", true, CardState::Live(&snapshot), None);
        assert_eq!(panel.sequence(0), 1);
        assert!(panel.pixels().contains(&FIRST));
        panel.begin(true, false);
        panel.card(
            0,
            "host",
            false,
            CardState::Retain,
            Some(Rates {
                received_fps: 60.0,
                ..Rates::default()
            }),
        );
        assert!(panel.pixels().contains(&FIRST));
        panel.invalidate(0);
        panel.card(0, "host", false, CardState::Retain, None);
        assert_eq!(panel.sequence(0), 0);
        assert!(!panel.pixels().contains(&FIRST));
        let mut surface = PresentationSurface::default();
        surface
            .update::<SurfaceError>(1, (2, 2), |pixels| {
                pixels.resize(4, NEXT);
                Ok(())
            })
            .unwrap();
        let snapshot = surface.snapshot_after(0).unwrap();
        panel.card(0, "host", true, CardState::Live(&snapshot), None);
        assert!(panel.pixels().contains(&NEXT));
        panel.card(0, "host", true, CardState::Waiting, None);
        assert!(!panel.pixels().contains(&NEXT));
    }
}
