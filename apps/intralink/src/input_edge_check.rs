//! Explicit, finite, read-only BASE edge calibration. This command owns no
//! network endpoint, keyboard hook, cursor warp or input-injection authority.
use super::{load_targets, portal::Mode, topology_canvas as ui, topology_store};
use crate::local::approve;
use anyhow::{Context, Result, ensure};
use intralink_core::{
    display_geometry,
    overview::{self, HEIGHT, Rect, WIDTH},
    physical_edge::EdgeDetector,
    screen_topology::Edge,
};
use intralink_input_win::{
    EntryPortal, PortalStyle, indicator_visible, physical_cursor, physical_primary,
};
use minifb::{Key, MouseButton, MouseMode, ScaleMode, Window, WindowOptions};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tokio::time::sleep;

const START: Rect = Rect {
    x: 32,
    y: 445,
    w: 425,
    h: 62,
};
const CLOSE: Rect = Rect {
    x: 503,
    y: 445,
    w: 425,
    h: 62,
};
const WINDOW_WIDTH: usize = 640;
const WINDOW_HEIGHT: usize = 360;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Start,
    Close,
}

#[derive(Default)]
pub(super) struct Click {
    neutral: bool,
    down: bool,
    pressed: Option<Action>,
}
impl Click {
    pub(super) fn sample(
        &mut self,
        active: bool,
        down: bool,
        action: Option<Action>,
    ) -> Option<Action> {
        if !active {
            *self = Self::default();
            return None;
        }
        if !self.neutral {
            self.neutral = !down;
            self.down = down;
            return None;
        }
        let mut result = None;
        if down && !self.down {
            self.pressed = action;
        } else if !down && self.down {
            result = self.pressed.take().filter(|p| Some(*p) == action);
        }
        self.down = down;
        result
    }
}

fn window() -> Result<Window> {
    let mut window = Window::new(
        "IntraLink | Physical edge check - READ ONLY",
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        WindowOptions {
            resize: false,
            topmost: true,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| anyhow::anyhow!("Edge check window: {e}"))?;
    window.set_target_fps(0);
    Ok(window)
}

struct Status<'a> {
    seconds: u64,
    running: bool,
    matched: u32,
    last: Option<(Edge, &'a str)>,
    size: (u16, u16),
}

fn render(canvas: &mut [u32], status: Status<'_>) {
    canvas.fill(ui::BG);
    ui::label(canvas, 32, 24, "INTRALINK / PHYSICAL EDGES", 3, ui::ACCENT);
    ui::label(canvas, 32, 79, "READ ONLY - NO REMOTE CONTROL", 2, ui::INK);
    ui::label(
        canvas,
        32,
        121,
        &format!("BASE {} X {} PHYSICAL PIXELS", status.size.0, status.size.1),
        2,
        ui::MUTED,
    );
    overview::fill(
        canvas,
        Rect {
            x: 32,
            y: 162,
            w: 896,
            h: 240,
        },
        ui::CARD,
    );
    ui::label(
        canvas,
        56,
        181,
        if status.running {
            "MOVE TO A SHARED OUTER EDGE"
        } else {
            "READY FOR LOCAL CALIBRATION"
        },
        2,
        ui::INK,
    );
    ui::label(
        canvas,
        56,
        222,
        &format!(
            "{} SEC LEFT / {} VALID PROPOSALS",
            status.seconds, status.matched
        ),
        2,
        ui::ACCENT,
    );
    if let Some((edge, name)) = status.last {
        ui::label(
            canvas,
            56,
            267,
            &format!("PROPOSED: {edge:?} TO {name}"),
            2,
            ui::ACCENT,
        );
    } else {
        ui::label(canvas, 56, 267, "NO EDGE PROPOSAL YET", 2, ui::MUTED);
    }
    ui::label(
        canvas,
        56,
        309,
        "APPROACH FROM INTERIOR / HOLD AT EDGE",
        1,
        ui::MUTED,
    );
    ui::label(
        canvas,
        56,
        334,
        "CORNERS AND GAPS NEVER SELECT A CLIENT",
        1,
        ui::MUTED,
    );
    ui::label(
        canvas,
        56,
        360,
        "NO POINTER HISTORY IS SAVED OR TRANSMITTED",
        1,
        ui::MUTED,
    );
    ui::button(
        canvas,
        START,
        if status.running {
            "CHECK RUNNING"
        } else {
            "START CHECK"
        },
        !status.running,
    );
    ui::button(canvas, CLOSE, "STOP / CLOSE", true);
}

fn hit(window: &Window, running: bool) -> Option<Action> {
    let point = window.get_mouse_pos(MouseMode::Discard)?;
    let (x, y) = overview::map_point(window.get_size(), point)?;
    if CLOSE.contains(x, y) {
        Some(Action::Close)
    } else if !running && START.contains(x, y) {
        Some(Action::Start)
    } else {
        None
    }
}

pub(crate) async fn run(path: &Path, names: &[String], seconds: u16) -> Result<()> {
    ensure!(
        (5..=120).contains(&seconds),
        "Edge check duration outside 5-120 seconds"
    );
    let (_vault, secret, targets) = load_targets(path, names, false, false, false, None).await?;
    let observed = topology_store::observe_base()?;
    let screen = physical_primary()?;
    ensure!(
        screen.size() == (observed.display.width, observed.display.height),
        "Physical and reported BASE dimensions disagree"
    );
    let peers = targets.iter().map(|(_, id, _, _)| *id).collect();
    let store = topology_store::Store::new(path, secret.public(), peers, observed)?;
    drop(secret);
    let config = store.load()?.context(
        "Save this exact device topology with input topology before physical edge calibration",
    )?;
    let topology =
        display_geometry::topology(&config.placements).context("Invalid saved topology")?;
    ensure!(
        topology.connected(),
        "Saved topology contains disconnected CLIENTs"
    );
    approve(
        "Run a read-only physical pointer edge check for the selected layout? While the visible window runs, only current cursor position and geometry are sampled in memory; no keyboard capture, cursor movement, network connection or remote activation. Close/minimize the window or stop to end. Click START CHECK to begin.",
    )?;
    let mut window = window()?;
    let mut canvas = vec![0; WIDTH * HEIGHT];
    let mut portal = match config.portal {
        Mode::Full => Some(EntryPortal::new(PortalStyle::Full)?),
        Mode::Reduced => Some(EntryPortal::new(PortalStyle::Reduced)?),
        Mode::Off => None,
    };
    let mut detector = EdgeDetector::default();
    let mut click = Click::default();
    let opened = Instant::now();
    let mut started = None;
    let mut check_at = opened;
    let mut matched = 0u32;
    let mut last = None;
    let mut painted = None;
    loop {
        let now = Instant::now();
        let remaining = started.map_or(u64::from(seconds), |start: Instant| {
            u64::from(seconds).saturating_sub(now.saturating_duration_since(start).as_secs())
        });
        if remaining == 0 || (started.is_none() && opened.elapsed() >= Duration::from_secs(60)) {
            break;
        }
        let running = started.is_some();
        let state = (running, remaining, matched, last);
        if painted != Some(state) {
            render(
                &mut canvas,
                Status {
                    seconds: remaining,
                    running,
                    matched,
                    last: last
                        .map(|(edge, index): (Edge, usize)| (edge, targets[index].0.as_str())),
                    size: screen.size(),
                },
            );
            window
                .update_with_buffer(&canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("Edge check: {e}"))?;
            painted = Some(state);
        } else {
            window.update();
        }
        if !window.is_open()
            || !indicator_visible(window.get_window_handle() as usize)
            || (window.is_active()
                && (window.is_key_down(Key::Escape) || window.is_key_down(Key::F12)))
        {
            break;
        }
        match click.sample(
            window.is_active(),
            window.get_mouse_down(MouseButton::Left),
            hit(&window, running),
        ) {
            Some(Action::Close) => break,
            Some(Action::Start) => {
                ensure!(
                    physical_primary()? == screen && topology_store::observe_base()? == observed,
                    "BASE display changed; review layout again"
                );
                detector.reset();
                started = Some(Instant::now());
                check_at = Instant::now();
            }
            None => {}
        }
        if let Some(start) = started {
            if check_at.elapsed() >= Duration::from_millis(500) {
                ensure!(
                    physical_primary()? == screen && topology_store::observe_base()? == observed,
                    "BASE geometry changed; calibration stopped"
                );
                check_at = Instant::now();
            }
            let point = physical_cursor()?;
            if let Some(proposal) =
                detector.sample(screen, observed.display, &topology, point, start.elapsed())
            {
                let index = usize::from(proposal.transfer.target)
                    .checked_sub(1)
                    .context("Invalid edge proposal")?;
                ensure!(
                    index < targets.len(),
                    "Edge proposal not in selected topology"
                );
                matched = matched.saturating_add(1);
                last = Some((proposal.edge, index));
                if let Some(portal) = portal.as_mut() {
                    portal.begin_at(screen, point, Instant::now())?;
                }
            }
            if let Some(portal) = portal.as_mut() {
                portal.poll(Instant::now())?;
            }
        }
        sleep(Duration::from_millis(16)).await;
    }
    drop(portal);
    println!(
        "Physical edge check closed; {matched} local proposals, no remote input or pointer history."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_start_requires_a_fresh_complete_focused_click() {
        let mut click = Click::default();
        assert_eq!(click.sample(true, true, Some(Action::Start)), None);
        assert_eq!(click.sample(true, false, Some(Action::Start)), None);
        assert_eq!(click.sample(true, true, Some(Action::Start)), None);
        assert_eq!(click.sample(false, false, Some(Action::Start)), None);
        assert_eq!(click.sample(true, false, Some(Action::Start)), None);
        assert_eq!(click.sample(true, true, Some(Action::Start)), None);
        assert_eq!(click.sample(true, false, Some(Action::Close)), None);
        assert_eq!(click.sample(true, true, Some(Action::Start)), None);
        assert_eq!(
            click.sample(true, false, Some(Action::Start)),
            Some(Action::Start)
        );
    }
    #[test]
    #[ignore = "requires local graphical desktop; does not sample cursor or open a network connection"]
    fn native_edge_check_indicator_is_visible_and_presents_read_only_state() -> Result<()> {
        let mut w = window()?;
        let mut canvas = vec![0; WIDTH * HEIGHT];
        render(
            &mut canvas,
            Status {
                seconds: 30,
                running: false,
                matched: 0,
                last: None,
                size: (3840, 2160),
            },
        );
        w.update_with_buffer(&canvas, WIDTH, HEIGHT)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        ensure!(
            indicator_visible(w.get_window_handle() as usize),
            "Expected visible local indicator"
        );
        render(
            &mut canvas,
            Status {
                seconds: 29,
                running: true,
                matched: 1,
                last: Some((Edge::Right, "GENERATED CLIENT")),
                size: (3840, 2160),
            },
        );
        for _ in 0..8 {
            w.update_with_buffer(&canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            assert!(w.is_open());
        }
        println!("NATIVE_EDGE_INDICATOR_PASS generated status; no sampling, no connection");
        Ok(())
    }
}
