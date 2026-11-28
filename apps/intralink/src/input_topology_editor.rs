//! Offline geometric topology editor. No endpoint, listener or capture.
use super::{
    portal::Mode,
    topology_canvas::{self as ui, Cards, EDIT_AREA, Projection},
    topology_store::{self, Config, Store},
};
use anyhow::{Context, Result, ensure};
use intralink_core::{
    display_geometry::{self, Display, Placement},
    overview::{self, HEIGHT, Rect, WIDTH},
};
use minifb::{Key, KeyRepeat, MouseButton};

const PREV: Rect = Rect {
    x: 640,
    y: 80,
    w: 126,
    h: 35,
};
const NEXT: Rect = Rect {
    x: 782,
    y: 80,
    w: 146,
    h: 35,
};
const APPLY: Rect = Rect {
    x: 640,
    y: 360,
    w: 288,
    h: 42,
};
const CANCEL: Rect = Rect {
    x: 16,
    y: 481,
    w: 126,
    h: 42,
};
const RESET: Rect = Rect {
    x: 154,
    y: 481,
    w: 126,
    h: 42,
};
const PORTAL: Rect = Rect {
    x: 292,
    y: 481,
    w: 164,
    h: 42,
};
const SAVE: Rect = Rect {
    x: 746,
    y: 481,
    w: 198,
    h: 42,
};
const LABELS: [&str; 6] = [
    "PIXEL WIDTH",
    "PIXEL HEIGHT",
    "SCALE %",
    "ROTATION",
    "LOGICAL X",
    "LOGICAL Y",
];
fn field(i: usize) -> Rect {
    Rect {
        x: 640 + (i % 2) * 148,
        y: 169 + (i / 2) * 62,
        w: 140,
        h: 34,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Card(u8),
    Field(usize),
    Prev,
    Next,
    Apply,
    Cancel,
    Reset,
    Portal,
    Save,
}

struct Editor {
    config: Config,
    confirmed: u16,
    selected: u8,
    fields: [String; 6],
    editing: Option<usize>,
    replace: bool,
    status: &'static str,
}

impl Editor {
    fn fresh(store: &Store) -> Self {
        // Clearly labelled placeholder geometry only. It cannot be saved
        // until every CLIENT is explicitly confirmed by Apply.
        let base = store.observed().display;
        let mut x = i32::from(base.logical().expect("validated BASE").0);
        let mut placements = vec![Placement {
            id: 0,
            x: 0,
            y: 0,
            display: base,
        }];
        for i in 1..store.slots() {
            placements.push(Placement {
                id: i as u8,
                x,
                y: 0,
                display: Display {
                    width: 1280,
                    height: 720,
                    scale: 100,
                    rotation: 0,
                },
            });
            x += 1280;
        }
        let mut editor = Self {
            config: Config {
                placements,
                portal: Mode::Full,
            },
            confirmed: 1,
            selected: 1,
            fields: std::array::from_fn(|_| String::new()),
            editing: None,
            replace: false,
            status: "CONFIRM THE ACTUAL CLIENT DIMENSIONS",
        };
        editor.select(1);
        editor
    }

    fn load(store: &Store) -> Self {
        let mut editor = Self::fresh(store);
        match store.load() {
            Ok(Some(config)) => {
                editor.config = config;
                editor.confirmed = editor.mask();
                editor.select(1);
                editor.status = "SAVED GEOMETRY RESTORED / REVIEW BEFORE SAVING";
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!(
                    "Saved topology not usable: {error:#}. Save explicitly replaces a regular preference file."
                );
                editor.status = "SAVED FILE INVALID / REVIEW AND SAVE TO REPLACE";
            }
        }
        editor
    }

    fn mask(&self) -> u16 {
        (1 << self.config.placements.len()) - 1
    }
    fn select(&mut self, id: u8) {
        let p = self.config.placements[usize::from(id)];
        self.selected = id;
        self.fields = [
            p.display.width.to_string(),
            p.display.height.to_string(),
            p.display.scale.to_string(),
            p.display.rotation.to_string(),
            p.x.to_string(),
            p.y.to_string(),
        ];
        self.editing = None;
        self.replace = false;
    }

    fn apply(&mut self) -> Result<()> {
        ensure!(self.selected != 0, "BASE geometry is measured, not edited");
        let p = Placement {
            id: self.selected,
            x: self.fields[4].parse()?,
            y: self.fields[5].parse()?,
            display: Display {
                width: self.fields[0].parse()?,
                height: self.fields[1].parse()?,
                scale: self.fields[2].parse()?,
                rotation: self.fields[3].parse()?,
            },
        };
        let mut trial = self.config.placements.clone();
        trial[usize::from(self.selected)] = p;
        let t = display_geometry::topology(&trial)
            .context("Invalid dimensions, position or overlapping displays")?;
        ensure!(
            Projection::new(&t, EDIT_AREA).is_some(),
            "Geometry too small for editor"
        );
        self.config.placements = trial;
        self.confirmed |= 1 << self.selected;
        self.select(self.selected);
        self.status = "CLIENT GEOMETRY CONFIRMED";
        Ok(())
    }

    fn drag(&mut self, id: u8, delta: (i32, i32), projection: Projection) -> Result<()> {
        let old = self.config.placements[usize::from(id)];
        let (dx, dy) = projection.delta(delta.0, delta.1);
        let tolerance = projection.delta(12, 0).0.unsigned_abs().min(16384);
        let next = display_geometry::snapped(
            &self.config.placements,
            id,
            (old.x + dx, old.y + dy),
            tolerance,
        )
        .context("Overlapping or invalid drop")?;
        let mut trial = self.config.placements.clone();
        trial[usize::from(id)] = next;
        let t = display_geometry::topology(&trial).context("Invalid drop")?;
        ensure!(
            Projection::new(&t, EDIT_AREA).is_some(),
            "Dropped topology cannot be displayed"
        );
        self.config.placements = trial;
        self.select(id);
        self.status = "POSITION UPDATED / TOUCH EDGES TO CONNECT";
        Ok(())
    }

    fn save_ready(&self) -> bool {
        self.confirmed == self.mask()
            && self.editing.is_none()
            && display_geometry::topology(&self.config.placements)
                .is_some_and(|t| t.connected() && Projection::new(&t, ui::LIVE_AREA).is_some())
    }

    fn key(&mut self, key: Key) -> bool {
        let Some(index) = self.editing else {
            return false;
        };
        let value = &mut self.fields[index];
        match key {
            Key::Key0
            | Key::NumPad0
            | Key::Key1
            | Key::NumPad1
            | Key::Key2
            | Key::NumPad2
            | Key::Key3
            | Key::NumPad3
            | Key::Key4
            | Key::NumPad4
            | Key::Key5
            | Key::NumPad5
            | Key::Key6
            | Key::NumPad6
            | Key::Key7
            | Key::NumPad7
            | Key::Key8
            | Key::NumPad8
            | Key::Key9
            | Key::NumPad9 => {
                if self.replace {
                    value.clear();
                    self.replace = false;
                }
                let digit = match key {
                    Key::Key0 | Key::NumPad0 => '0',
                    Key::Key1 | Key::NumPad1 => '1',
                    Key::Key2 | Key::NumPad2 => '2',
                    Key::Key3 | Key::NumPad3 => '3',
                    Key::Key4 | Key::NumPad4 => '4',
                    Key::Key5 | Key::NumPad5 => '5',
                    Key::Key6 | Key::NumPad6 => '6',
                    Key::Key7 | Key::NumPad7 => '7',
                    Key::Key8 | Key::NumPad8 => '8',
                    _ => '9',
                };
                if value.len() < 7 {
                    value.push(digit);
                }
            }
            Key::Minus | Key::NumPadMinus if index >= 4 => {
                if self.replace {
                    value.clear();
                    self.replace = false;
                }
                if value.is_empty() {
                    value.push('-');
                }
            }
            Key::Backspace | Key::Delete => {
                if self.replace || key == Key::Delete {
                    value.clear();
                    self.replace = false;
                } else {
                    value.pop();
                }
            }
            Key::Tab => {
                self.editing = Some((index + 1) % 6);
                self.replace = true;
            }
            _ => return false,
        }
        self.status = "EDITING / CLICK APPLY TO CONFIRM";
        true
    }

    fn hit(
        &self,
        projection: Projection,
        t: &intralink_core::screen_topology::Topology,
        at: (usize, usize),
    ) -> Option<Action> {
        if let Some(sample) = projection.sample(t, at) {
            return Some(Action::Card(sample.id));
        }
        if self.selected != 0 {
            for i in 0..6 {
                if field(i).contains(at.0, at.1) {
                    return Some(Action::Field(i));
                }
            }
            if APPLY.contains(at.0, at.1) {
                return Some(Action::Apply);
            }
        }
        [
            (PREV, Action::Prev),
            (NEXT, Action::Next),
            (CANCEL, Action::Cancel),
            (RESET, Action::Reset),
            (PORTAL, Action::Portal),
            (SAVE, Action::Save),
        ]
        .into_iter()
        .find(|(r, _)| r.contains(at.0, at.1))
        .map(|(_, a)| a)
    }

    fn render(
        &self,
        canvas: &mut [u32],
        names: &[&str],
        t: &intralink_core::screen_topology::Topology,
        projection: Projection,
    ) {
        canvas.fill(ui::BG);
        ui::label(
            canvas,
            16,
            13,
            "INTRALINK / DISPLAY TOPOLOGY",
            2,
            ui::ACCENT,
        );
        ui::label(
            canvas,
            16,
            38,
            "DRAG CLIENTS TO TOUCH EDGES / CONFIGURE EXACT PIXELS AND SCALE",
            1,
            ui::MUTED,
        );
        overview::fill(canvas, EDIT_AREA, 0x182638);
        ui::render_cards(
            canvas,
            t,
            projection,
            Cards {
                names,
                placements: &self.config.placements,
                confirmed: self.confirmed,
                selected: Some(self.selected),
                ready: 0,
                active: None,
                editing: true,
            },
        );
        ui::label(canvas, 640, 57, "SELECT DISPLAY", 2, ui::INK);
        ui::button(canvas, PREV, "PREV", true);
        ui::button(canvas, NEXT, "NEXT", true);
        ui::label(
            canvas,
            640,
            125,
            names[usize::from(self.selected)],
            1,
            ui::ACCENT,
        );
        for (i, name) in LABELS.iter().enumerate() {
            let r = field(i);
            ui::label(canvas, r.x, r.y - 17, name, 1, ui::MUTED);
            overview::fill(
                canvas,
                r,
                if self.editing == Some(i) {
                    0x174c40
                } else {
                    ui::CARD
                },
            );
            ui::label(canvas, r.x + 8, r.y + 9, &self.fields[i], 2, ui::INK);
        }
        if self.selected == 0 {
            ui::label(canvas, 640, 366, "BASE MEASURED / LOCKED", 2, ui::ACCENT);
        } else {
            ui::button(canvas, APPLY, "APPLY CLIENT", true);
        }
        ui::label(
            canvas,
            640,
            413,
            if t.connected() {
                "ALL EDGES LINKED"
            } else {
                "GAPS / UNLINKED SCREENS"
            },
            1,
            if t.connected() { ui::ACCENT } else { 0xffbd70 },
        );
        ui::label(
            canvas,
            640,
            429,
            &format!(
                "{} / {} CONFIRMED",
                self.confirmed.count_ones(),
                self.config.placements.len()
            ),
            1,
            ui::MUTED,
        );
        ui::label(canvas, 16, 463, self.status, 1, ui::INK);
        ui::button(canvas, CANCEL, "CANCEL", true);
        ui::button(canvas, RESET, "RESET", true);
        ui::button(canvas, PORTAL, self.config.portal.label(), true);
        ui::button(canvas, SAVE, "SAVE / REVIEW", self.save_ready());
    }
}

fn ghost_rect(rect: Rect, delta: (i32, i32)) -> Option<Rect> {
    let x = rect.x as i32 + delta.0;
    let y = rect.y as i32 + delta.1;
    let left = x.max(EDIT_AREA.x as i32);
    let top = y.max(EDIT_AREA.y as i32);
    let right = (x + rect.w as i32).min((EDIT_AREA.x + EDIT_AREA.w) as i32);
    let bottom = (y + rect.h as i32).min((EDIT_AREA.y + EDIT_AREA.h) as i32);
    (right - left >= 2 && bottom - top >= 2).then_some(Rect {
        x: left as usize,
        y: top as usize,
        w: (right - left) as usize,
        h: (bottom - top) as usize,
    })
}

fn draw_ghost(canvas: &mut [u32], rect: Rect) {
    for edge in [
        Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: 1,
        },
        Rect {
            x: rect.x,
            y: rect.y + rect.h - 1,
            w: rect.w,
            h: 1,
        },
        Rect {
            x: rect.x,
            y: rect.y,
            w: 1,
            h: rect.h,
        },
        Rect {
            x: rect.x + rect.w - 1,
            y: rect.y,
            w: 1,
            h: rect.h,
        },
    ] {
        overview::fill(canvas, edge, ui::ACCENT);
    }
}

/// Explicit Save writes the identity-bound local preference. Escape, cancel,
/// focus loss during a click, or invalid/unconfirmed geometry never saves.
pub(super) fn edit(store: &Store, names: &[&str]) -> Result<Config> {
    ensure!(names.len() == store.slots(), "Missing topology names");
    let mut editor = Editor::load(store);
    let mut window = super::window(
        "IntraLink | Configure displays (offline)",
        WIDTH,
        HEIGHT,
        false,
        ui::BG,
    )?;
    window.set_target_fps(30);
    let mut canvas = vec![0; WIDTH * HEIGHT];
    let mut down = true;
    let mut pressed = None;
    let mut dirty = true;
    let mut t =
        display_geometry::topology(&editor.config.placements).context("Invalid editor state")?;
    let mut projection = Projection::new(&t, EDIT_AREA).context("Editor topology too small")?;
    let mut geometry_dirty = false;
    let mut ghost = None;
    while window.is_open() && !window.is_key_down(Key::Escape) {
        if geometry_dirty {
            t = display_geometry::topology(&editor.config.placements)
                .context("Invalid editor state")?;
            projection = Projection::new(&t, EDIT_AREA).context("Editor topology too small")?;
            geometry_dirty = false;
        }
        if dirty {
            editor.render(&mut canvas, names, &t, projection);
            if let Some(rect) = ghost {
                draw_ghost(&mut canvas, rect);
            }
            window
                .update_with_buffer(&canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("Topology editor: {e}"))?;
            dirty = false;
        } else {
            window.update();
        }
        if !window.is_open() || window.is_key_down(Key::Escape) {
            break;
        }
        let point = super::pad::pointer(&window);
        let now_down = window.get_mouse_down(MouseButton::Left);
        if !window.is_active() {
            pressed = None;
            if ghost.take().is_some() {
                dirty = true;
            }
            down = true;
            continue;
        }
        if !now_down {
            for key in window.get_keys_pressed(KeyRepeat::Yes) {
                if editor.key(key) {
                    dirty = true;
                }
            }
        }
        if now_down && !down {
            pressed = point.and_then(|at| {
                editor
                    .hit(projection, &t, at)
                    .map(|action| (action, at, projection))
            });
        } else if !now_down
            && down
            && let (Some((action, start, projection)), Some(end)) = (pressed.take(), point)
        {
            let at = editor.hit(projection, &t, end);
            if let Action::Card(id) = action {
                if id != 0 && EDIT_AREA.contains(end.0, end.1) && start != end {
                    if editor
                        .drag(
                            id,
                            (end.0 as i32 - start.0 as i32, end.1 as i32 - start.1 as i32),
                            projection,
                        )
                        .is_err()
                    {
                        editor.status = "DROP REJECTED / KEEP SCREENS NONOVERLAPPING";
                    } else {
                        geometry_dirty = true;
                    }
                } else if at == Some(action) {
                    editor.select(id);
                }
            } else if at == Some(action) {
                match action {
                    Action::Field(i) => {
                        editor.editing = Some(i);
                        editor.replace = true;
                    }
                    Action::Prev => editor.select(
                        (usize::from(editor.selected) + store.slots() - 1).rem_euclid(store.slots())
                            as u8,
                    ),
                    Action::Next => editor
                        .select((usize::from(editor.selected) + 1).rem_euclid(store.slots()) as u8),
                    Action::Apply => {
                        if editor.apply().is_err() {
                            editor.status = "INVALID VALUES OR OVERLAP / ADJUST POSITION";
                        } else {
                            geometry_dirty = true;
                        }
                    }
                    Action::Cancel => break,
                    Action::Reset => {
                        editor = Editor::fresh(store);
                        geometry_dirty = true;
                    }
                    Action::Portal => editor.config.portal = editor.config.portal.next(),
                    Action::Save => {
                        if !editor.save_ready() {
                            editor.status = "CONFIRM ALL CLIENTS AND CONNECT THEIR EDGES";
                        } else if topology_store::observe_base()? != store.observed() {
                            anyhow::bail!("BASE monitor changed; reopen topology editor");
                        } else {
                            store.save(&editor.config)?;
                            return Ok(editor.config);
                        }
                    }
                    _ => {}
                }
            }
            dirty = true;
        }
        // A clipped, local ghost follows the held pointer. This never changes
        // the geometry or hit testing before the drop is validated.
        let next_ghost = if now_down {
            match (pressed, point) {
                (Some((Action::Card(id), start, projection)), Some(end))
                    if id != 0 && start != end =>
                {
                    let screen = t
                        .screens()
                        .iter()
                        .find(|s| s.id == id)
                        .copied()
                        .expect("selected card");
                    ghost_rect(
                        projection.rect(screen),
                        (end.0 as i32 - start.0 as i32, end.1 as i32 - start.1 as i32),
                    )
                }
                _ => None,
            }
        } else {
            None
        };
        if ghost != next_ghost {
            ghost = next_ghost;
            dirty = true;
        }
        if !now_down {
            pressed = None;
        }
        down = now_down;
    }
    anyhow::bail!("Topology editing canceled; no session opened")
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;
    #[test]
    #[ignore = "requires local native graphical desktop; generated peer only"]
    fn native_topology_editor_observes_base_and_presents_generated_layout() -> Result<()> {
        let observed = topology_store::observe_base()?;
        let dir = tempfile::tempdir()?;
        let id = |n: u8| SecretKey::from_bytes(&[n; 32]).public();
        let store = Store::new(&dir.path().join("test"), id(1), vec![id(2)], observed)?;
        let mut editor = Editor::fresh(&store);
        let mut canvas = vec![0; WIDTH * HEIGHT];
        let mut window = super::super::window(
            "IntraLink | Generated geometry test",
            WIDTH,
            HEIGHT,
            false,
            ui::BG,
        )?;
        window.set_target_fps(30);
        for confirmed in [false, true] {
            if confirmed {
                // Generated portrait CLIENT fixture; only BASE is observed.
                editor.fields[0] = "1080".into();
                editor.fields[1] = "1920".into();
                editor.fields[2] = "125".into();
                editor.fields[3] = "90".into();
                editor.fields[4] = "-864".into();
                editor.fields[5] = "0".into();
                editor.apply()?;
                assert!(editor.save_ready());
                store.save(&editor.config)?;
                assert!(store.load()?.is_some());
            }
            let topology =
                display_geometry::topology(&editor.config.placements).context("Test topology")?;
            let projection = Projection::new(&topology, EDIT_AREA).context("Test projection")?;
            editor.render(
                &mut canvas,
                &["BASE", "GENERATED CLIENT"],
                &topology,
                projection,
            );
            for _ in 0..8 {
                window
                    .update_with_buffer(&canvas, WIDTH, HEIGHT)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                assert!(window.is_open());
            }
        }
        println!(
            "NATIVE_TOPOLOGY_PASS base={}x{} scale={} rotation={} submissions=16; generated CLIENT only",
            observed.display.width,
            observed.display.height,
            observed.display.scale,
            observed.display.rotation
        );
        Ok(())
    }

    #[test]
    fn placeholders_cannot_be_saved_and_edits_are_atomic() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let id = |n: u8| SecretKey::from_bytes(&[n; 32]).public();
        let store = Store::new(
            &dir.path().join("test"),
            id(1),
            vec![id(2)],
            topology_store::Observed {
                monitor: 1,
                display: Display {
                    width: 1920,
                    height: 1080,
                    scale: 100,
                    rotation: 0,
                },
            },
        )?;
        let mut e = Editor::fresh(&store);
        assert!(!e.save_ready());
        e.fields[0] = "1080".into();
        e.fields[1] = "1920".into();
        e.fields[2] = "125".into();
        e.fields[3] = "90".into();
        e.fields[4] = "-864".into();
        e.fields[5] = "100".into();
        e.apply()?;
        assert!(e.save_ready());
        let previous = e.config.placements[1];
        e.fields[4] = "0".into();
        assert!(e.apply().is_err());
        assert_eq!(e.config.placements[1], previous);
        e.select(1);
        e.editing = Some(0);
        e.replace = true;
        e.key(Key::Key1);
        assert!(!e.save_ready());
        e.select(0);
        assert!(e.apply().is_err());
        assert!(store.load()?.is_none());
        Ok(())
    }
}
