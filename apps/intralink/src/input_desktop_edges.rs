//! Physical-edge handoff with a temporary, explicitly visible foreground
//! surface. No global hooks. The normal desktop is untouched in LOCAL mode.
use super::{
    Slot, Snapshot,
    edge_check::{Action, Click},
    keys, local_buttons, topology_canvas as ui,
    topology_store::{self, Config, Observed},
};
use anyhow::{Context, Result, ensure};
use intralink_core::{
    display_geometry,
    input::{Event, Focus, Key as RemoteKey},
    overview::{self, HEIGHT, Rect, WIDTH},
    physical_edge::{EdgeDetector, PhysicalScreen},
    screen_topology::Transfer,
    virtual_pointer::{Motion, VirtualPointer},
};
use intralink_input_win::{
    ForegroundCapture, indicator_visible, neutral_for_handoff, physical_cursor, physical_primary,
    with_physical_dpi,
};
use minifb::{Key, MouseButton, MouseMode, ScaleMode, Window, WindowOptions};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
use tokio::time::sleep;

const POLL: Duration = Duration::from_millis(8);
const SESSION_LIMIT: Duration = Duration::from_secs(300);
const ARM_LIMIT: Duration = Duration::from_secs(60);
const MAX_MOTION_GAP: Duration = Duration::from_millis(100);
const START: Rect = Rect {
    x: 32,
    y: 420,
    w: 425,
    h: 82,
};
const CLOSE: Rect = Rect {
    x: 503,
    y: 420,
    w: 425,
    h: 82,
};

fn stop_requested(window: &Window) -> bool {
    !window.is_open() || window.is_key_down(Key::F12) || window.is_key_down(Key::Escape)
}
fn neutral(window: &Window) -> bool {
    window.get_keys().is_empty() && local_buttons(window).is_empty() && neutral_for_handoff()
}
fn supported_keys(window: &Window) -> Result<BTreeSet<RemoteKey>> {
    window
        .get_keys()
        .into_iter()
        .map(|key| {
            keys::desktop_key(key)
                .context("Unsupported or OS-reserved key; desktop preview stopped")
        })
        .collect()
}

fn indicator() -> Result<Window> {
    let mut window = Window::new(
        "IntraLink | Physical desktop KVM - SAFE",
        640,
        360,
        WindowOptions {
            topmost: true,
            resize: false,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| anyhow::anyhow!("Desktop KVM indicator: {e}"))?;
    window.set_target_fps(0);
    Ok(window)
}

fn paint_indicator(canvas: &mut [u32], armed: bool, remaining: u64) {
    canvas.fill(ui::BG);
    ui::label(
        canvas,
        32,
        32,
        "INTRALINK / PHYSICAL DESKTOP KVM",
        3,
        ui::ACCENT,
    );
    ui::label(
        canvas,
        32,
        95,
        if armed {
            "LOCAL / ARMED"
        } else {
            "LOCAL / SAFE"
        },
        3,
        ui::INK,
    );
    ui::label(
        canvas,
        32,
        151,
        &format!("{} SECONDS LEFT", remaining),
        2,
        ui::MUTED,
    );
    ui::label(
        canvas,
        32,
        211,
        "MOVE TO AN APPROVED SHARED EDGE",
        2,
        ui::INK,
    );
    ui::label(
        canvas,
        32,
        256,
        "CAPTURE SURFACE OPENS ONLY WHILE REMOTE",
        2,
        ui::MUTED,
    );
    ui::label(
        canvas,
        32,
        299,
        "F12 OR ESC STOPS WHILE CONTROL WINDOW IS FOCUSED",
        1,
        ui::MUTED,
    );
    ui::label(
        canvas,
        32,
        338,
        "MINIMIZE OR CLOSE THIS INDICATOR TO STOP",
        1,
        ui::MUTED,
    );
    ui::button(
        canvas,
        START,
        if armed { "ARMED" } else { "ARM EDGES" },
        !armed,
    );
    ui::button(canvas, CLOSE, "STOP / CLOSE", true);
}
fn hit(window: &Window, armed: bool) -> Option<Action> {
    let (x, y) = overview::map_point(window.get_size(), window.get_mouse_pos(MouseMode::Discard)?)?;
    if CLOSE.contains(x, y) {
        Some(Action::Close)
    } else if !armed && START.contains(x, y) {
        Some(Action::Start)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
struct SessionPermit {
    indicator: usize,
    started: Instant,
}

impl SessionPermit {
    fn valid(self) -> bool {
        self.started.elapsed() < SESSION_LIMIT && indicator_visible(self.indicator)
    }
}

struct Surface {
    // The buffer outlives the native window, including its final WM_PAINT.
    window: Window,
    native: ForegroundCapture,
    canvas: Vec<u32>,
    pointer: VirtualPointer,
    previous: Snapshot,
    sampled_at: Instant,
}

impl Surface {
    async fn open(
        screen: PhysicalScreen,
        pointer: VirtualPointer,
        name: &str,
        slots: &[Slot],
        permit: SessionPermit,
    ) -> Result<Self> {
        ensure!(
            permit.valid() && neutral_for_handoff(),
            "Release all local keys and buttons before transfer"
        );
        let mut canvas = vec![0; WIDTH * HEIGHT];
        let mut window = with_physical_dpi(|| {
            Window::new(
                "IntraLink | Remote input - F12 or Escape to stop",
                WIDTH,
                HEIGHT,
                WindowOptions {
                    borderless: true,
                    none: true,
                    title: false,
                    topmost: true,
                    resize: false,
                    scale_mode: ScaleMode::AspectRatioStretch,
                    ..WindowOptions::default()
                },
            )
            .map_err(|e| anyhow::anyhow!("Desktop capture window: {e}"))
        })?;
        window.set_target_fps(0);
        paint_capture(&mut canvas, name, false);
        window
            .update_with_buffer(&canvas, WIDTH, HEIGHT)
            .map_err(|e| anyhow::anyhow!("Desktop capture: {e}"))?;
        let native = ForegroundCapture::prepare(window.get_window_handle() as usize, screen)?;
        let focused = native.request_focus();
        window.update();
        if !focused || !window.is_active() || !native.ready() {
            // Windows may deny a foreground request from the background.
            // Require a fresh local confirmation. Never relay input here.
            paint_focus_confirmation(&mut canvas, name);
            window
                .update_with_buffer(&canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("Foreground confirmation: {e}"))?;
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut click = Click::default();
            loop {
                window.update();
                ensure!(
                    !stop_requested(&window)
                        && native.geometry_matches()
                        && permit.valid()
                        && slots.iter().all(Slot::connected),
                    "Foreground confirmation canceled or CLIENT disconnected"
                );
                ensure!(
                    Instant::now() < deadline,
                    "Windows foreground confirmation timed out; no CLIENT activated"
                );
                let at = overview::map_point(
                    window.get_size(),
                    window
                        .get_mouse_pos(MouseMode::Discard)
                        .unwrap_or((-1.0, -1.0)),
                );
                let action = at.and_then(|(x, y)| {
                    if START.contains(x, y) {
                        Some(Action::Start)
                    } else if CLOSE.contains(x, y) {
                        Some(Action::Close)
                    } else {
                        None
                    }
                });
                match click.sample(
                    window.is_active() && native.ready(),
                    window.get_mouse_down(MouseButton::Left),
                    action,
                ) {
                    Some(Action::Start) if neutral(&window) && native.owns_current_pointer() => {
                        break;
                    }
                    Some(Action::Close) => anyhow::bail!("Foreground confirmation canceled"),
                    _ => {}
                }
                sleep(POLL).await;
            }
            paint_capture(&mut canvas, name, false);
            window
                .update_with_buffer(&canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("Desktop capture: {e}"))?;
        }
        ensure!(
            physical_primary()? == screen,
            "Physical BASE monitor changed during confirmation"
        );
        ensure!(
            permit.valid()
                && window.is_active()
                && !stop_requested(&window)
                && neutral(&window)
                && native.owns_current_pointer(),
            "Windows foreground capture refused or input changed"
        );
        // No cursor movement until the remote activation and entry Move ACK.
        Ok(Self {
            window,
            native,
            canvas,
            pointer,
            previous: Snapshot::default(),
            sampled_at: Instant::now(),
        })
    }
    fn display(&mut self, name: &str) -> Result<()> {
        paint_capture(&mut self.canvas, name, true);
        self.window
            .update_with_buffer(&self.canvas, WIDTH, HEIGHT)
            .map_err(|e| anyhow::anyhow!("Desktop capture: {e}"))
    }
}

fn paint_focus_confirmation(canvas: &mut [u32], name: &str) {
    canvas.fill(ui::BG);
    ui::label(
        canvas,
        32,
        32,
        "INTRALINK / LOCAL CONFIRMATION",
        3,
        ui::ACCENT,
    );
    ui::label(
        canvas,
        32,
        112,
        "WINDOWS REQUIRES YOUR FOCUS CONFIRMATION",
        2,
        ui::INK,
    );
    ui::label(canvas, 32, 175, name, 3, ui::ACCENT);
    ui::label(
        canvas,
        32,
        250,
        "CLICK CONFIRM TO HAND OFF / NO REMOTE INPUT YET",
        2,
        ui::MUTED,
    );
    ui::label(
        canvas,
        32,
        320,
        "8 SECOND LIMIT / ESC OR F12 CANCELS",
        2,
        0xffbd70,
    );
    ui::button(canvas, START, "CONFIRM", true);
    ui::button(canvas, CLOSE, "CANCEL", true);
}

fn paint_capture(canvas: &mut [u32], name: &str, active: bool) {
    canvas.fill(ui::BG);
    ui::label(canvas, 32, 30, "INTRALINK / REMOTE INPUT", 3, ui::ACCENT);
    ui::label(
        canvas,
        32,
        110,
        if active {
            "CONTROL SURFACE ACTIVE"
        } else {
            "WAITING FOR CLIENT ACK"
        },
        3,
        ui::INK,
    );
    ui::label(canvas, 32, 177, name, 3, ui::ACCENT);
    ui::label(
        canvas,
        32,
        260,
        if active {
            "YOUR MOUSE AND KEYBOARD CONTROL THE CLIENT"
        } else {
            "NO REMOTE INPUT UNTIL ACKNOWLEDGED"
        },
        2,
        ui::INK,
    );
    ui::label(
        canvas,
        32,
        306,
        "MOVE THROUGH ITS SHARED EDGE TO RETURN",
        2,
        ui::MUTED,
    );
    ui::label(
        canvas,
        32,
        410,
        "F12 / ESC - STOP AND RETURN TO LOCAL",
        3,
        0xffbd70,
    );
    ui::label(
        canvas,
        32,
        475,
        "SYSTEM SHORTCUTS AND UNSUPPORTED KEYS STOP THE PREVIEW",
        1,
        ui::MUTED,
    );
}

fn check_geometry(
    config: &Config,
    observed: Observed,
    screen: PhysicalScreen,
    slots: &[Slot],
) -> Result<()> {
    ensure!(
        config.placements.len() == slots.len() + 1
            && slots
                .iter()
                .zip(config.placements.iter().skip(1))
                .all(|(s, p)| s.display() == Some(p.display)),
        "Selected CLIENT geometry session ended"
    );
    ensure!(
        physical_primary()? == screen && topology_store::observe_base()? == observed,
        "BASE physical monitor changed"
    );
    Ok(())
}

/// The capture surface must own the actual foreground and the pointer at
/// every ACK barrier. A failure closes the workspace and its remote workers.
async fn transfer(
    focus: &mut Focus,
    slots: &[Slot],
    surface: &mut Surface,
    next: Transfer,
    permit: SessionPermit,
) -> Result<()> {
    let target = usize::from(next.target).checked_sub(1);
    if let Some(index) = target {
        ensure!(
            slots.get(index).is_some_and(Slot::connected),
            "Destination CLIENT unavailable"
        );
    }
    let native = &surface.native;
    let guard = |window: &Window| {
        permit.valid()
            && native.owns_current_pointer()
            && !stop_requested(window)
            && neutral(window)
    };
    if let Some(old) = focus.begin(target)? {
        slots[old]
            .confirmed_guarded(Event::Deactivate, &mut surface.window, guard)
            .await?;
    }
    ensure!(focus.released()? == target, "Unexpected release target");
    if let Some(index) = target {
        slots[index]
            .confirmed_guarded(Event::Activate, &mut surface.window, guard)
            .await?;
        slots[index]
            .confirmed_guarded(
                Event::Move {
                    x: next.x,
                    y: next.y,
                },
                &mut surface.window,
                guard,
            )
            .await?;
    }
    focus.activated(target)?;
    surface.previous = Snapshot::default();
    surface.previous.position = Some((next.x, next.y));
    if target.is_some() {
        // Discard motion during the acknowledged switch, not after it.
        ensure!(
            permit.valid() && neutral(&surface.window),
            "Local input changed at arrival"
        );
        surface.native.center()?;
        surface.sampled_at = Instant::now();
    }
    Ok(())
}

pub(super) async fn run(config: Config, observed: Observed, slots: &[Slot]) -> Result<()> {
    let screen = physical_primary()?;
    ensure!(
        screen.size() == (observed.display.width, observed.display.height),
        "Physical BASE dimensions disagree with reviewed geometry"
    );
    let topology =
        display_geometry::topology(&config.placements).context("Invalid reviewed geometry")?;
    ensure!(
        topology.connected(),
        "All CLIENTs must share a path to BASE"
    );
    check_geometry(&config, observed, screen, slots)?;
    let mut canvas = vec![0; WIDTH * HEIGHT];
    let mut indicator = indicator()?;
    let mut armed = None;
    let opened = Instant::now();
    let mut last_check = opened;
    let mut painted = None;
    let mut click = Click::default();
    let mut detector = EdgeDetector::default();
    let mut focus = Focus::default();
    let mut active: Option<Surface> = None;
    loop {
        let now = Instant::now();
        let elapsed = armed.map_or(Duration::ZERO, |start: Instant| {
            now.saturating_duration_since(start)
        });
        if elapsed >= SESSION_LIMIT || (armed.is_none() && now - opened >= ARM_LIMIT) {
            break;
        }
        if !indicator_visible(indicator.get_window_handle() as usize) || !indicator.is_open() {
            break;
        }
        ensure!(
            slots
                .iter()
                .zip(config.placements.iter().skip(1))
                .all(|(s, p)| s.display() == Some(p.display)),
            "A selected CLIENT disconnected"
        );
        if now - last_check >= Duration::from_millis(500) {
            check_geometry(&config, observed, screen, slots)?;
            last_check = Instant::now();
        }
        if let Some(surface) = active.as_mut() {
            // The foreground capture pumps its own events; the indicator
            // remains independently visible to the OS and can be closed.
            surface.window.update();
            if stop_requested(&surface.window) {
                break;
            }
            ensure!(
                surface.window.is_active() && surface.native.owns_current_pointer(),
                "Foreground capture lost; releasing all remote input"
            );
            let id = surface.pointer.id();
            let index = usize::from(id)
                .checked_sub(1)
                .context("Invalid virtual owner")?;
            ensure!(focus.active() == Some(index), "Remote input owner mismatch");
            let keys = supported_keys(&surface.window)?;
            let can_cross = keys.is_empty()
                && local_buttons(&surface.window).is_empty()
                && surface.previous.keys.is_empty()
                && surface.previous.buttons.is_empty();
            ensure!(
                surface.sampled_at.elapsed() <= MAX_MOTION_GAP,
                "Physical pointer sampling stalled; releasing remote input"
            );
            let delta = surface.native.motion()?;
            surface.sampled_at = Instant::now();
            // Check global neutrality only if crossing is about to happen.
            let motion = surface
                .pointer
                .motion(&topology, observed.display, delta, can_cross, elapsed)
                .context("Invalid relative pointer sample")?;
            match motion {
                Motion::Move { x, y } => {
                    ensure!(
                        surface.window.is_active() && surface.native.owns_current_pointer(),
                        "Capture ownership changed before forwarding"
                    );
                    surface.previous.forward_with_keys(
                        &surface.window,
                        &slots[index],
                        (x, y),
                        keys,
                    )?;
                }
                Motion::Cross(next) => {
                    if !neutral(&surface.window) {
                        // OS-reserved/unknown held state: do not attempt a
                        // transfer or replay a motion while uncertain.
                        anyhow::bail!("Local input changed during physical handoff");
                    }
                    let permit = SessionPermit {
                        indicator: indicator.get_window_handle() as usize,
                        started: armed.context("Missing armed session")?,
                    };
                    transfer(&mut focus, slots, surface, next, permit).await?;
                    if next.target == 0 {
                        surface.native.return_at(next.x, next.y)?;
                        // Release ACK precedes local pointer placement and
                        // destruction of the foreground surface.
                        drop(active.take());
                        detector.reset();
                        click = Click::default();
                        painted = None;
                    } else {
                        surface.pointer =
                            VirtualPointer::enter(&topology, next, armed.expect("armed").elapsed())
                                .context("Invalid acknowledged arrival")?;
                        surface.display(&slots[usize::from(next.target) - 1].name)?;
                        surface.sampled_at = Instant::now();
                    }
                }
            }
        } else {
            let remaining = SESSION_LIMIT.saturating_sub(elapsed).as_secs();
            let state = (armed.is_some(), remaining);
            if painted != Some(state) {
                paint_indicator(&mut canvas, state.0, state.1);
                indicator
                    .update_with_buffer(&canvas, WIDTH, HEIGHT)
                    .map_err(|e| anyhow::anyhow!("Desktop indicator: {e}"))?;
                painted = Some(state);
            } else {
                indicator.update();
            }
            if !indicator.is_open()
                || !indicator_visible(indicator.get_window_handle() as usize)
                || (indicator.is_active() && stop_requested(&indicator))
            {
                break;
            }
            match click.sample(
                indicator.is_active(),
                indicator.get_mouse_down(MouseButton::Left),
                hit(&indicator, armed.is_some()),
            ) {
                Some(Action::Close) => break,
                Some(Action::Start) if neutral(&indicator) => {
                    check_geometry(&config, observed, screen, slots)?;
                    detector.reset();
                    armed = Some(Instant::now());
                }
                _ => {}
            }
            if let Some(start) = armed {
                let point = physical_cursor()?;
                if let Some(proposal) =
                    detector.sample(screen, observed.display, &topology, point, start.elapsed())
                {
                    if neutral_for_handoff() {
                        check_geometry(&config, observed, screen, slots)?;
                        let next = proposal.transfer;
                        let pointer = VirtualPointer::enter(&topology, next, start.elapsed())
                            .context("Invalid BASE edge entry")?;
                        let index = usize::from(next.target) - 1;
                        ensure!(
                            slots.get(index).is_some_and(Slot::connected),
                            "Proposed CLIENT unavailable"
                        );
                        let permit = SessionPermit {
                            indicator: indicator.get_window_handle() as usize,
                            started: start,
                        };
                        let mut surface =
                            Surface::open(screen, pointer, &slots[index].name, slots, permit)
                                .await?;
                        check_geometry(&config, observed, screen, slots)?;
                        ensure!(
                            permit.valid(),
                            "Desktop-edge session ended during confirmation"
                        );
                        transfer(&mut focus, slots, &mut surface, next, permit).await?;
                        surface.pointer = VirtualPointer::enter(&topology, next, start.elapsed())
                            .context("Invalid acknowledged entry")?;
                        surface.display(&slots[index].name)?;
                        surface.sampled_at = Instant::now();
                        active = Some(surface);
                    } else {
                        detector.reset();
                    }
                }
            }
        }
        sleep(POLL).await;
    }
    // Normal emergency stop or indicator closure: destroy the owned surface
    // immediately and let the outer workspace abort/close all QUIC workers.
    // A clean edge return above instead waits for Deactivate ACK.
    drop(active);
    drop(indicator);
    println!("Physical desktop KVM stopped; closing remote input sessions.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_key_policy_does_not_forward_system_or_stop_keys() {
        for key in [
            Key::F12,
            Key::Escape,
            Key::LeftAlt,
            Key::RightAlt,
            Key::LeftSuper,
            Key::RightSuper,
            Key::CapsLock,
            Key::NumLock,
            Key::ScrollLock,
        ] {
            assert!(keys::desktop_key(key).is_none());
        }
        assert_eq!(keys::desktop_key(Key::F1).unwrap().usage(), 58);
        assert_eq!(keys::desktop_key(Key::F9).unwrap().usage(), 66);
        assert_eq!(keys::desktop_key(Key::A).unwrap().usage(), 4);
    }

    #[test]
    #[ignore = "requires local native desktop; small generated window, no input or warp"]
    fn native_foreground_source_verifies_only_its_owned_window_geometry() -> Result<()> {
        let primary = physical_primary()?;
        let (width, height) = primary.size();
        ensure!(
            width >= 800 && height >= 500,
            "Native test needs a sufficiently large monitor"
        );
        let (x, y) = primary.origin();
        let rectangle = PhysicalScreen::new(x + 40, y + 40, 640, 360).context("Test rectangle")?;
        let mut canvas = vec![0; WIDTH * HEIGHT];
        let mut w = with_physical_dpi(|| {
            Window::new(
                "IntraLink | Native geometry test - NO INPUT",
                640,
                360,
                WindowOptions {
                    borderless: true,
                    none: true,
                    title: false,
                    topmost: true,
                    resize: false,
                    scale_mode: ScaleMode::AspectRatioStretch,
                    ..WindowOptions::default()
                },
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
        })?;
        w.set_target_fps(0);
        paint_capture(&mut canvas, "GENERATED WINDOW", false);
        w.update_with_buffer(&canvas, WIDTH, HEIGHT)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let guard = ForegroundCapture::prepare(w.get_window_handle() as usize, rectangle)?;
        w.update();
        assert!(guard.geometry_matches());
        let granted = guard.request_focus();
        w.update();
        if granted {
            ensure!(guard.ready(), "Granted test foreground disappeared");
        } else {
            assert!(!guard.ready());
        }
        drop(w);
        assert!(!guard.ready() && !guard.geometry_matches());
        // No calls to motion(), center(), return_at(), or input workers.
        println!(
            "NATIVE_FOREGROUND_GEOMETRY_PASS 640x360 owned window, foreground={granted}; no cursor movement or input"
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires native local graphical desktop; no capture or input"]
    fn native_desktop_edge_surfaces_present_generated_status() -> Result<()> {
        let mut canvas = vec![0; WIDTH * HEIGHT];
        let mut w = indicator()?;
        paint_indicator(&mut canvas, false, 300);
        w.update_with_buffer(&canvas, WIDTH, HEIGHT)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        ensure!(
            indicator_visible(w.get_window_handle() as usize),
            "Missing local status window"
        );
        paint_capture(&mut canvas, "GENERATED CLIENT", true);
        for _ in 0..8 {
            w.update_with_buffer(&canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(w.is_open(), "Status window closed");
        }
        // Deliberately never call ForegroundCapture::prepare or cursor placement.
        drop(w);
        println!(
            "NATIVE_DESKTOP_EDGE_STATUS_PASS generated UI only; no capture, warp, listener or connection"
        );
        Ok(())
    }
}
