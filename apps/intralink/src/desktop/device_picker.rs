//! Foreground selector for paired screen or input hosts. Discovery hints
//! never grant authority; each capability uses its own pinned handshake.
use super::overview::text;
use crate::discovery::Hints;
#[cfg(all(target_os = "windows", feature = "input-preview"))]
use crate::discovery::{self, Protocol};
use anyhow::{Context, Result, ensure};
use intralink_core::{
    input::MAX_SLOTS,
    overview::{self, HEIGHT, Rect, WIDTH},
};
use intralink_vault::{Direction, Profile};
use iroh::EndpointId;
use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};
use std::collections::BTreeSet;
use tokio::task::JoinHandle;

const PAGE_SIZE: usize = 8;
const BG: u32 = 0x101923;
const CARD: u32 = 0x223245;
const ACCENT: u32 = 0x63ddc2;
const INK: u32 = 0xeff5fa;
const MUTED: u32 = 0xa8b9ca;
const BACK: Rect = Rect {
    x: 32,
    y: 475,
    w: 110,
    h: 42,
};
const PREVIOUS: Rect = Rect {
    x: 160,
    y: 475,
    w: 104,
    h: 42,
};
const NEXT: Rect = Rect {
    x: 278,
    y: 475,
    w: 104,
    h: 42,
};
const REFRESH: Rect = Rect {
    x: 402,
    y: 475,
    w: 144,
    h: 42,
};
const START: Rect = Rect {
    x: 748,
    y: 475,
    w: 180,
    h: 42,
};

struct Device {
    name: String,
    id: EndpointId,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Device(usize),
    Back,
    Previous,
    Next,
    Refresh,
    Start,
}

fn card(slot: usize) -> Rect {
    Rect {
        x: 32 + (slot % 2) * 456,
        y: 106 + (slot / 2) * 88,
        w: 440,
        h: 76,
    }
}

struct Selection {
    count: usize,
    limit: usize,
    page: usize,
    selected: BTreeSet<usize>,
}

impl Selection {
    fn new(count: usize) -> Self {
        Self {
            count,
            limit: MAX_SLOTS,
            page: 0,
            selected: BTreeSet::new(),
        }
    }

    fn pages(&self) -> usize {
        self.count.div_ceil(PAGE_SIZE).max(1)
    }

    fn toggle(&mut self, index: usize) {
        if index >= self.count {
            return;
        }
        if self.selected.remove(&index) {
            return;
        }
        if self.limit == 1 {
            self.selected.clear();
        }
        if self.selected.len() < self.limit {
            self.selected.insert(index);
        }
    }

    fn enabled(&self, action: Action, scanning: bool) -> bool {
        match action {
            Action::Device(index) => {
                index < self.count
                    && (self.limit == 1
                        || self.selected.contains(&index)
                        || self.selected.len() < self.limit)
            }
            Action::Back => true,
            Action::Previous => self.page > 0,
            Action::Next => self.page + 1 < self.pages(),
            Action::Refresh => !scanning,
            Action::Start => !scanning && !self.selected.is_empty(),
        }
    }

    fn hit(&self, x: usize, y: usize) -> Option<Action> {
        for slot in 0..PAGE_SIZE {
            let index = self.page * PAGE_SIZE + slot;
            if index < self.count && card(slot).contains(x, y) {
                return Some(Action::Device(index));
            }
        }
        [
            (BACK, Action::Back),
            (PREVIOUS, Action::Previous),
            (NEXT, Action::Next),
            (REFRESH, Action::Refresh),
            (START, Action::Start),
        ]
        .into_iter()
        .find(|(rect, _)| rect.contains(x, y))
        .map(|(_, action)| action)
    }
}

/// A complete local click is required; focus loss cancels pending gestures.
#[derive(Default)]
struct Click {
    neutral: bool,
    down: bool,
    pressed: Option<Action>,
}

impl Click {
    fn step(&mut self, focused: bool, down: bool, hit: Option<Action>) -> Option<Action> {
        if !focused {
            *self = Self::default();
            return None;
        }
        if !self.neutral {
            self.neutral = !down;
            self.down = down;
            return None;
        }
        let mut action = None;
        if down && !self.down {
            self.pressed = hit;
        } else if !down && self.down {
            action = self.pressed.take().filter(|a| Some(*a) == hit);
        }
        self.down = down;
        action
    }
}

#[derive(Clone, Copy)]
enum Scan {
    SavedRoutes,
    Running,
    Complete,
    Failed,
}

impl Scan {
    fn label(self) -> &'static str {
        match self {
            Self::SavedRoutes => "SCREEN ONLY / EACH CLIENT MUST APPROVE ITS SESSION",
            Self::Running => "LOOKING FOR INPUT HOSTS ON YOUR LAN",
            Self::Complete => "LAN SCAN COMPLETE / HINTS ARE NOT AUTHENTICATION",
            Self::Failed => "LAN SCAN FAILED / SAVED ROUTES REMAIN AVAILABLE",
        }
    }
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

fn button(canvas: &mut [u32], rect: Rect, value: &str, enabled: bool) {
    overview::fill(canvas, rect, if enabled { 0x08685c } else { 0x35404e });
    let len = value.len().min(rect.w / 12);
    text(
        canvas,
        (rect.x + (rect.w - len * 12) / 2, rect.y + 14),
        value,
        2,
        len,
        INK,
    );
}

fn render(
    canvas: &mut [u32],
    devices: &[Device],
    selection: &Selection,
    hints: &Hints,
    scan: Scan,
) {
    canvas.fill(BG);
    label(
        canvas,
        32,
        15,
        if matches!(scan, Scan::SavedRoutes) {
            if selection.limit == 1 {
                "INTRALINK / SCREEN + INPUT"
            } else {
                "INTRALINK / SCREEN WORKSPACE"
            }
        } else {
            "INTRALINK / INPUT WORKSPACE"
        },
        2,
        ACCENT,
    );
    label(canvas, 32, 41, "SELECT YOUR DEVICES", 3, INK);
    label(
        canvas,
        32,
        72,
        if selection.limit == 1 {
            "SELECT ONE CLIENT / INDEPENDENT APPROVALS"
        } else {
            scan.label()
        },
        2,
        MUTED,
    );
    for (slot, device) in devices
        .iter()
        .skip(selection.page * PAGE_SIZE)
        .take(PAGE_SIZE)
        .enumerate()
    {
        let rect = card(slot);
        let index = selection.page * PAGE_SIZE + slot;
        let selected = selection.selected.contains(&index);
        overview::fill(canvas, rect, if selected { 0x174c40 } else { CARD });
        overview::fill(
            canvas,
            Rect {
                x: rect.x,
                y: rect.y,
                w: 4,
                h: rect.h,
            },
            if selected { ACCENT } else { 0x526779 },
        );
        // Validated peer labels are ASCII, at most 48 bytes.
        let name = &device.name;
        let scale = if name.len() <= 32 { 2 } else { 1 };
        text(
            canvas,
            (rect.x + 14, rect.y + 12),
            name,
            scale,
            (rect.w - 52) / (6 * scale),
            INK,
        );
        let status = if matches!(scan, Scan::SavedRoutes) {
            if selection.limit == 1 {
                "SAVED ID / SEPARATE SESSION APPROVALS"
            } else {
                "SAVED ROUTE / SCREEN APPROVAL REQUIRED"
            }
        } else if hints.contains_key(&device.id) {
            "INPUT ANNOUNCEMENT FOUND"
        } else if matches!(scan, Scan::Running) {
            "CHECKING LAN"
        } else {
            "NO ANNOUNCEMENT / SAVED ROUTE"
        };
        text(
            canvas,
            (rect.x + 14, rect.y + 42),
            status,
            1,
            60,
            if hints.contains_key(&device.id) {
                ACCENT
            } else {
                MUTED
            },
        );
        if selected {
            label(canvas, rect.x + rect.w - 26, rect.y + 12, "X", 2, ACCENT);
        }
    }
    label(
        canvas,
        32,
        461,
        &format!(
            "{} / {} SELECTED   PAGE {} / {}",
            selection.selected.len(),
            selection.limit,
            selection.page + 1,
            selection.pages()
        ),
        1,
        MUTED,
    );
    button(canvas, BACK, "CANCEL", true);
    button(canvas, PREVIOUS, "PREV", selection.page > 0);
    button(canvas, NEXT, "NEXT", selection.page + 1 < selection.pages());
    if !matches!(scan, Scan::SavedRoutes) {
        button(canvas, REFRESH, "REFRESH", !matches!(scan, Scan::Running));
    }
    button(
        canvas,
        START,
        "CONTINUE",
        !selection.selected.is_empty() && !matches!(scan, Scan::Running),
    );
}

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Screen,
    #[cfg(all(target_os = "windows", feature = "input-preview"))]
    Control,
    #[cfg(all(target_os = "windows", feature = "input-preview"))]
    Input(Protocol),
}

impl Kind {
    fn scan(self, _devices: &[Device]) -> Option<JoinHandle<Result<Hints>>> {
        match self {
            Self::Screen => None,
            #[cfg(all(target_os = "windows", feature = "input-preview"))]
            Self::Control => None,
            #[cfg(all(target_os = "windows", feature = "input-preview"))]
            Self::Input(protocol) => {
                let ids = _devices.iter().map(|device| device.id).collect::<Vec<_>>();
                Some(tokio::spawn(async move {
                    discovery::find_many(&ids, protocol).await
                }))
            }
        }
    }
}

/// The caller holds the appropriate profile/capability lease. No session
/// endpoint is created by the selector; each capability keeps its own policy.
pub(crate) async fn choose(profile: &Profile, kind: Kind) -> Result<(Vec<String>, Hints)> {
    super::require_graphical_terminal()?;
    let devices = profile
        .peers()
        .iter()
        .filter(|p| p.direction() == Direction::Outbound)
        .map(|p| {
            Ok(Device {
                name: p.name().to_owned(),
                id: p.identity()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        !devices.is_empty(),
        "Pair a CLIENT before opening the workspace"
    );
    let mut window = Window::new(
        "IntraLink | Select paired devices",
        WIDTH,
        HEIGHT,
        WindowOptions::default(),
    )
    .map_err(|e| anyhow::anyhow!("Device selector: {e}"))?;
    window.set_target_fps(30);
    let mut canvas = vec![0; WIDTH * HEIGHT];
    let mut selection = Selection::new(devices.len());
    #[cfg(all(target_os = "windows", feature = "input-preview"))]
    if matches!(kind, Kind::Control) {
        selection.limit = 1;
    }
    let mut hints = Hints::new();
    let mut worker = kind.scan(&devices);
    let mut scan = if worker.is_some() {
        Scan::Running
    } else {
        Scan::SavedRoutes
    };
    let mut click = Click::default();
    let mut dirty = true;
    let result = async {
        while window.is_open() && !window.is_key_down(Key::Escape) {
            if worker.as_ref().is_some_and(|task| task.is_finished()) {
                match worker.take().context("Missing discovery worker")?.await {
                    Ok(Ok(found)) => {
                        hints = found;
                        scan = Scan::Complete;
                    }
                    _ => {
                        hints.clear();
                        scan = Scan::Failed;
                    }
                }
                dirty = true;
            }
            if dirty {
                render(&mut canvas, &devices, &selection, &hints, scan);
                window
                    .update_with_buffer(&canvas, WIDTH, HEIGHT)
                    .map_err(|e| anyhow::anyhow!("Device selector: {e}"))?;
                dirty = false;
            } else {
                window.update();
            }
            if !window.is_open() || window.is_key_down(Key::Escape) {
                break;
            }
            let at = window
                .get_mouse_pos(MouseMode::Discard)
                .and_then(|point| overview::map_point(window.get_size(), point))
                .and_then(|(x, y)| selection.hit(x, y))
                .filter(|action| {
                    !(matches!(scan, Scan::SavedRoutes) && *action == Action::Refresh)
                        && selection.enabled(*action, matches!(scan, Scan::Running))
                });
            match click.step(
                window.is_active(),
                window.get_mouse_down(MouseButton::Left),
                at,
            ) {
                Some(Action::Device(index)) => {
                    selection.toggle(index);
                    dirty = true;
                }
                Some(Action::Previous) if selection.page > 0 => {
                    selection.page -= 1;
                    dirty = true;
                }
                Some(Action::Next) if selection.page + 1 < selection.pages() => {
                    selection.page += 1;
                    dirty = true;
                }
                Some(Action::Refresh) if worker.is_none() => {
                    hints.clear();
                    scan = Scan::Running;
                    worker = kind.scan(&devices);
                    dirty = true;
                }
                Some(Action::Start) if !selection.selected.is_empty() && worker.is_none() => {
                    return Ok(selection
                        .selected
                        .iter()
                        .map(|&i| devices[i].name.clone())
                        .collect::<Vec<_>>());
                }
                Some(Action::Back) => break,
                _ => {}
            }
            tokio::task::yield_now().await;
        }
        anyhow::bail!("Device selection canceled; no session was opened")
    }
    .await;
    drop(window);
    // Let the bounded scan shut down its owned daemon on cancellation too.
    if let Some(task) = worker {
        let _ = task.await;
    }
    Ok((result?, hints))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    #[test]
    fn pagination_and_selection_are_bounded_and_stable() {
        let mut selection = Selection::new(128);
        for index in 0..10 {
            selection.toggle(index);
        }
        assert_eq!(selection.selected.len(), 8);
        selection.toggle(128);
        assert_eq!(selection.selected.len(), 8);
        selection.toggle(0);
        selection.page = 15;
        assert_eq!(
            selection.hit(card(7).x, card(7).y),
            Some(Action::Device(127))
        );
        selection.toggle(127);
        assert!(selection.selected.contains(&127));
        assert_eq!(selection.pages(), 16);
        assert_eq!(selection.selected.len(), 8);
        assert_eq!(selection.hit(0, 0), None);
        assert!(!selection.enabled(Action::Start, true));
        assert!(selection.enabled(Action::Start, false));
        assert!(!selection.enabled(Action::Device(126), false));
        assert!(selection.enabled(Action::Device(127), false));
        assert!(!selection.enabled(Action::Next, false));
        let mut click = Click::default();
        assert_eq!(click.step(true, false, None), None);
        let disabled = Some(Action::Start).filter(|a| selection.enabled(*a, true));
        assert_eq!(click.step(true, true, disabled), None);
        assert_eq!(click.step(true, false, Some(Action::Start)), None);
    }

    #[test]
    #[ignore = "requires a native graphical desktop; generated identities only"]
    fn native_device_selector_presents_generated_cards() -> Result<()> {
        super::super::require_graphical_environment()?;
        let devices: Vec<_> = (1..=9)
            .map(|i| Device {
                name: format!("test-client-{i}"),
                id: SecretKey::from_bytes(&[i; 32]).public(),
            })
            .collect();
        let mut selection = Selection::new(devices.len());
        let mut canvas = vec![0; WIDTH * HEIGHT];
        let mut window = Window::new(
            "IntraLink | Generated selector test",
            WIDTH,
            HEIGHT,
            WindowOptions::default(),
        )
        .map_err(|e| anyhow::anyhow!("Device selector test: {e}"))?;
        window.set_target_fps(30);
        render(
            &mut canvas,
            &devices,
            &selection,
            &Hints::new(),
            Scan::Running,
        );
        for _ in 0..8 {
            window
                .update_with_buffer(&canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            assert!(window.is_open());
        }
        render(
            &mut canvas,
            &devices,
            &selection,
            &Hints::new(),
            Scan::SavedRoutes,
        );
        for _ in 0..8 {
            window
                .update_with_buffer(&canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            assert!(window.is_open());
        }
        let mut hints = Hints::new();
        hints.insert(devices[0].id, vec!["192.168.10.2:45100".parse()?]);
        selection.toggle(0);
        selection.toggle(8);
        selection.page = 1;
        render(&mut canvas, &devices, &selection, &hints, Scan::Complete);
        for _ in 0..8 {
            window
                .update_with_buffer(&canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            assert!(window.is_open());
        }
        println!("DEVICE_PICKER_NATIVE_PASS 8+8+8 submissions; generated cards only");
        Ok(())
    }

    #[test]
    fn screen_mode_never_starts_input_discovery_or_displays_refresh() {
        let devices = vec![Device {
            name: "screen".into(),
            id: SecretKey::from_bytes(&[9; 32]).public(),
        }];
        assert!(Kind::Screen.scan(&devices).is_none());
        let mut canvas = vec![0; WIDTH * HEIGHT];
        let selection = Selection::new(1);
        render(
            &mut canvas,
            &devices,
            &selection,
            &Hints::new(),
            Scan::SavedRoutes,
        );
        assert_eq!(canvas[REFRESH.y * WIDTH + REFRESH.x], BG);
    }

    #[test]
    fn selector_draws_actual_known_peers_and_rejects_stale_clicks() {
        let id = SecretKey::from_bytes(&[1; 32]).public();
        let devices = vec![Device {
            name: "test-client".to_owned(),
            id,
        }];
        let mut selection = Selection::new(1);
        let mut canvas = vec![0; WIDTH * HEIGHT];
        render(
            &mut canvas,
            &devices,
            &selection,
            &Hints::new(),
            Scan::Running,
        );
        let pos = card(0).y * WIDTH + card(0).x;
        assert_eq!(canvas[pos], 0x526779);
        selection.toggle(0);
        render(
            &mut canvas,
            &devices,
            &selection,
            &Hints::new(),
            Scan::Complete,
        );
        assert_eq!(canvas[pos], ACCENT);
        let mut click = Click::default();
        assert_eq!(click.step(true, true, Some(Action::Start)), None);
        assert_eq!(click.step(true, false, Some(Action::Start)), None);
        assert_eq!(click.step(true, true, Some(Action::Start)), None);
        assert_eq!(click.step(false, false, Some(Action::Start)), None);
        assert_eq!(click.step(true, false, Some(Action::Start)), None);
        assert_eq!(click.step(true, true, Some(Action::Start)), None);
        assert_eq!(click.step(true, false, Some(Action::Back)), None);
        assert_eq!(click.step(true, true, Some(Action::Start)), None);
        assert_eq!(
            click.step(true, false, Some(Action::Start)),
            Some(Action::Start)
        );
    }
}

#[cfg(test)]
#[test]
fn unified_selector_replaces_the_previous_single_selection() {
    let mut selection = Selection::new(12);
    selection.limit = 1;
    selection.toggle(1);
    assert!(selection.enabled(Action::Device(9), false));
    selection.toggle(9);
    assert_eq!(
        selection.selected.iter().copied().collect::<Vec<_>>(),
        vec![9]
    );
    selection.toggle(9);
    assert!(selection.selected.is_empty());
}
