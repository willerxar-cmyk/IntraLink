//! One foreground window; independent authorized view sessions and decoder workers.
use anyhow::{Context, Result, anyhow, ensure};
use intralink_core::{
    adaptive::is_keyframe,
    desktop::{CodecMode, ViewPriority},
    overview::{self, Action},
    presentation::PresentationSurface,
    telemetry::{Counters, RateSampler, SessionCounters},
    workspace::{MAX_DEVICES, WorkspacePolicy},
};
use intralink_transport::{
    bind_workspace_controller,
    desktop::{Stats, VideoFrame, ViewMode, view_with_observations},
};
use intralink_vault::{Direction, Route, SessionKind, SessionVault};
use iroh::{Endpoint, EndpointId};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, ScaleMode, Window, WindowOptions};
use std::{
    path::Path,
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use super::{
    codec::Decoder,
    monitors,
    overview::{CardState, Overview},
    require_graphical_terminal,
    source_status::SourceStatus,
};
use crate::{local::prompt_password, routing};

const KEYS: [Key; MAX_DEVICES] = [
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
];

pub(super) struct Target {
    pub(super) name: String,
    pub(super) identity: EndpointId,
    pub(super) route: Route,
    pub(super) mode: ViewMode,
}

pub(super) struct Session {
    stop: Option<oneshot::Sender<()>>,
    pub(super) task: JoinHandle<Result<Option<Stats>>>,
    pub(super) surface: Arc<Mutex<PresentationSurface>>,
    background: Arc<AtomicBool>,
    counters: Arc<SessionCounters>,
    pub(super) source_status: Arc<SourceStatus>,
    sampler: RateSampler,
    started: Instant,
    last_presented_sequence: usize,
}

impl Session {
    pub(super) fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }

    fn set_priority(&self, priority: ViewPriority) {
        self.background
            .store(priority == ViewPriority::Background, Ordering::Relaxed);
    }
}

// Each session has a bounded ordered queue and one independent decode worker.
// Only fully decoded pixels are shared with the foreground window; no delta is dropped.
pub(super) fn start(target: &Target, endpoint: Endpoint, priority: ViewPriority) -> Session {
    let mode = target.mode;
    let identity = target.identity;
    let route = target.route.clone();
    let (stop, mut cancelled) = oneshot::channel();
    let surface = Arc::new(Mutex::new(PresentationSurface::default()));
    let decode_surface = surface.clone();
    let background = Arc::new(AtomicBool::new(priority == ViewPriority::Background));
    let feedback = background.clone();
    let counters = Arc::new(SessionCounters::default());
    let receive_counters = counters.clone();
    let decode_counters = counters.clone();
    let source_status = Arc::new(SourceStatus::default());
    let observed = source_status.clone();
    let started = Instant::now();
    let task = tokio::spawn(async move {
        let candidates = tokio::select! {
            biased;
            _ = &mut cancelled => return Ok(None),
            result = routing::resolve(&route) => result?,
        };
        let (frames, mut pending) = mpsc::channel::<VideoFrame>(1);
        let (recycle, mut recycled) = mpsc::channel::<Vec<u8>>(2);
        let mut decoder_task = tokio::task::spawn_blocking(move || -> Result<()> {
            let mut decoder = Decoder::default();
            while let Some(mut frame) = pending.blocking_recv() {
                let mut surface = decode_surface
                    .lock()
                    .map_err(|_| anyhow!("Media surface lock poisoned"))?;
                let dimensions = (
                    frame.config.width() as usize,
                    frame.config.height() as usize,
                );
                let decoding = Instant::now();
                let decode = |pixels: &mut Vec<u32>| {
                    decoder.decode_into_protocol(
                        frame.config,
                        &frame.payload,
                        pixels,
                        mode.protocol,
                    )
                };
                if mode.protocol.is_adaptive() && is_keyframe(frame.config.mode(), &frame.payload) {
                    surface.update_keyframe(frame.sequence, dimensions, decode)?;
                } else {
                    surface.update(frame.sequence, dimensions, decode)?;
                }
                drop(surface);
                decode_counters.decoded(decoding.elapsed());
                frame.payload.clear();
                let _ = recycle.try_send(frame.payload);
            }
            Ok(())
        });
        let (result, worker_done) = {
            let network = view_with_observations(
                &endpoint,
                identity,
                &candidates,
                mode,
                || recycled.try_recv().unwrap_or_default(),
                move |frame| {
                    receive_counters.received(frame.payload.len());
                    let frames = frames.clone();
                    async move {
                        frames
                            .send(frame)
                            .await
                            .map_err(|_| anyhow!("Workspace decoder stopped"))
                    }
                },
                move |record| {
                    observed.observe(record);
                    if feedback.load(Ordering::Relaxed) {
                        ViewPriority::Background
                    } else {
                        ViewPriority::Foreground
                    }
                },
            );
            tokio::pin!(network);
            tokio::select! {
                biased;
                _ = &mut cancelled => (Ok(None), false),
                result = &mut network => (result.map(Some), false),
                result = &mut decoder_task => {
                    let error = match result {
                        Ok(Err(error)) => error,
                        Err(error) => anyhow!("Decoder worker failed: {error}"),
                        Ok(Ok(())) => anyhow!("Decoder stopped before transport completed"),
                    };
                    (Err(error), true)
                }
            }
        }; // dropping network releases its sender and any in-progress operation
        // The viewer guard closes only its connection. The workspace owns
        // the shared endpoint; stopping this slot must never close it.
        if !worker_done {
            decoder_task
                .await
                .context("Workspace decoder join failed")??;
        }
        result
    });
    Session {
        stop: Some(stop),
        task,
        surface,
        background,
        counters,
        source_status,
        sampler: RateSampler::new(Duration::ZERO, Counters::default()),
        started,
        last_presented_sequence: 0,
    }
}

#[derive(Clone, Copy)]
struct MonitorRect {
    x: isize,
    y: isize,
    width: usize,
    height: usize,
}

fn monitor_rect(id: Option<u32>) -> Result<MonitorRect> {
    let monitor = monitors()?
        .into_iter()
        .find(|monitor| match id {
            Some(id) => monitor.id().ok() == Some(id),
            None => monitor.is_primary().ok() == Some(true),
        })
        .context("Selected local monitor unavailable")?;
    let width = monitor
        .width()
        .map_err(|error| anyhow!("Monitor width: {error}"))? as usize;
    let height = monitor
        .height()
        .map_err(|error| anyhow!("Monitor height: {error}"))? as usize;
    ensure!(width > 0 && height > 0, "Invalid local monitor size");
    Ok(MonitorRect {
        x: monitor.x().map_err(|error| anyhow!("Monitor x: {error}"))? as isize,
        y: monitor.y().map_err(|error| anyhow!("Monitor y: {error}"))? as isize,
        width,
        height,
    })
}

fn open_window(fullscreen: bool, monitor_id: Option<u32>) -> Result<Window> {
    let rect = if fullscreen {
        Some(monitor_rect(monitor_id)?)
    } else {
        None
    };
    let (w, h) = rect.map_or((1280, 720), |r| (r.width, r.height));
    let mut window = Window::new(
        "IntraLink Workspace",
        w,
        h,
        WindowOptions {
            borderless: fullscreen,
            title: !fullscreen,
            resize: !fullscreen,
            topmost: fullscreen,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .map_err(|error| anyhow!("Workspace window: {error}"))?;
    if let Some(rect) = rect {
        window.set_position(rect.x, rect.y);
    }
    // Pace after releasing the shared canvas, never while holding its lock.
    window.set_target_fps(0);
    Ok(window)
}

/// Cycle the *local* presentation monitor; never request another host display.
fn next_local_monitor(current: Option<u32>) -> Result<Option<u32>> {
    let displays = monitors()?;
    let primary = displays
        .iter()
        .find(|monitor| monitor.is_primary().ok() == Some(true))
        .and_then(|monitor| monitor.id().ok());
    let mut ids: Vec<u32> = displays
        .iter()
        .filter_map(|monitor| monitor.id().ok())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() < 2 {
        return Ok(None);
    }
    let next = current
        .or(primary)
        .and_then(|id| ids.iter().position(|candidate| *candidate == id))
        .map_or(0, |position| (position + 1) % ids.len());
    Ok(Some(ids[next]))
}

// Exercise the production window factory and overview painter without
// opening a vault, capturing the desktop, or initiating a network session.
pub fn check_native_windows() -> Result<()> {
    super::require_graphical_environment()?;
    let mut panel = Overview::new(3).context("Invalid diagnostic layout")?;
    let mut window = open_window(false, None)?;
    check_window(&mut window, &mut panel, false, None, "windowed")?;

    let primary = monitor_rect(None)?;
    let mut next = open_window(true, None)?;
    check_window(
        &mut next,
        &mut panel,
        true,
        Some((primary.width, primary.height)),
        "primary fullscreen",
    )?;
    window = next;

    if let Some(id) = next_local_monitor(None)? {
        let rect = monitor_rect(Some(id))?;
        let mut next = open_window(true, Some(id))?;
        check_window(
            &mut next,
            &mut panel,
            true,
            Some((rect.width, rect.height)),
            "secondary fullscreen",
        )?;
        window = next;
    } else {
        println!("Secondary fullscreen: skipped (one available display)");
    }

    let mut next = open_window(false, None)?;
    check_window(&mut next, &mut panel, false, None, "restored windowed")?;
    drop(window);
    println!("Native UI diagnostic passed; no capture or network session performed.");
    Ok(())
}

fn check_window(
    window: &mut Window,
    panel: &mut Overview,
    fullscreen: bool,
    expected_size: Option<(usize, usize)>,
    label: &str,
) -> Result<()> {
    panel.begin(false, fullscreen);
    for (index, name) in ["LOCAL CHECK 1", "LOCAL CHECK 2", "LOCAL CHECK 3"]
        .iter()
        .enumerate()
    {
        panel.card(index, name, index == 0, CardState::Inactive, None);
    }
    window.set_title("IntraLink | Local UI diagnostic - no screen capture");
    window
        .update_with_buffer(panel.pixels(), overview::WIDTH, overview::HEIGHT)
        .map_err(|error| anyhow!("Native diagnostic presentation: {error}"))?;
    ensure!(window.is_open(), "Diagnostic window closed");
    let size = window.get_size();
    ensure!(size.0 > 0 && size.1 > 0, "Invalid native window geometry");
    if let Some(expected) = expected_size {
        ensure!(
            size == expected,
            "{label}: expected {expected:?}, got {size:?}"
        );
    }
    println!("{label}: {} x {}, native buffer accepted", size.0, size.1);
    Ok(())
}

fn show_selection(window: &mut Window, targets: &[Target], policy: &WorkspacePolicy) {
    let slot = policy.selected();
    let name = &targets[slot].name;
    let active = if policy.is_active(slot) {
        "active"
    } else {
        "inactive"
    };
    let title = format!(
        "IntraLink | F{}: {name} ({active}) | TAB devices, F9 toggle, F10 all-live, F11 fullscreen, F12 local display, ESC exit",
        slot + 1
    );
    window.set_title(&title);
    println!("{title}");
}

fn update_priorities(sessions: &[Option<Session>], policy: &WorkspacePolicy) {
    for (index, session) in sessions.iter().enumerate() {
        if let (Some(session), Some(priority)) = (session, policy.priority(index)) {
            session.set_priority(priority);
        }
    }
}

pub enum Targets {
    Explicit(Vec<String>, Vec<ViewMode>),
    Choose {
        limits: super::ReceiveProfileArgs,
        codec: CodecMode,
        adaptive: bool,
        negotiated: bool,
    },
}

/// Every target is drawn from an existing outbound grant. Hold the screen
/// lane and shared profile lease until all owned sessions have closed.
pub async fn run(
    path: &Path,
    selected: Targets,
    mut fullscreen: bool,
    mut monitor_id: Option<u32>,
    all_live: bool,
) -> Result<()> {
    if let Targets::Explicit(names, modes) = &selected {
        ensure!(
            modes.len() == names.len(),
            "Missing workspace receiver profiles"
        );
        WorkspacePolicy::new(names.len(), all_live).context("Select 1-8 distinct hosts")?;
    }
    require_graphical_terminal()?;
    let vault = SessionVault::lock(path, SessionKind::Screen)?;
    let password = prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    drop(password);
    ensure!(
        profile.role().can_initiate(),
        "This profile cannot initiate sessions"
    );
    let (names, modes) = match selected {
        Targets::Explicit(names, modes) => (names, modes),
        Targets::Choose {
            limits,
            codec,
            adaptive,
            negotiated,
        } => {
            let (names, _) =
                super::device_picker::choose(&profile, super::device_picker::Kind::Screen).await?;
            let modes = limits.workspace_modes(codec, adaptive, negotiated, &names, &[])?;
            (names, modes)
        }
    };
    let mut policy =
        WorkspacePolicy::new(names.len(), all_live).context("Select 1-8 distinct hosts")?;
    let mut targets: Vec<Target> = Vec::with_capacity(names.len());
    for (name, mode) in names.iter().zip(modes) {
        let peer = profile.peer(name, Direction::Outbound)?;
        let identity = peer.identity()?;
        ensure!(
            !targets.iter().any(|target| target.identity == identity),
            "Duplicate workspace host"
        );
        targets.push(Target {
            name: name.clone(),
            identity,
            route: peer.route().context("Outbound route missing")?.clone(),
            mode,
        });
    }
    let secret = profile.secret_key();
    drop(profile);
    let mut overview_panel =
        Overview::new(targets.len()).context("Invalid device overview layout")?;
    let mut window = open_window(fullscreen, monitor_id)?;
    println!(
        "Each Host must locally authorize its own view session. No automatic reconnect or input control."
    );
    for (index, target) in targets.iter().enumerate() {
        println!(
            "F{}: {} (receiver ceiling {}p/{} FPS)",
            index + 1,
            target.name,
            target.mode.limits.height(),
            target.mode.limits.fps()
        );
    }
    let endpoint = bind_workspace_controller(secret).await?;
    let mut sessions: Vec<Option<Session>> = targets
        .iter()
        .enumerate()
        .map(|(index, target)| {
            Some(start(
                target,
                endpoint.clone(),
                policy.priority(index).expect("Initially active"),
            ))
        })
        .collect();
    show_selection(&mut window, &targets, &policy);
    let black = [0u32; 4];
    let mut presented: Option<(usize, usize, (usize, usize), bool)> = None;
    let mut banner_pixels = Vec::new();
    let mut overview_open = false;
    let mut last_overview_paint: Option<Instant> = None;
    let mut last_metrics_sample = Instant::now();
    let mut left_down = false;
    let result = async {
        while window.is_open() {
            let tick = Instant::now();
            let mut overview_dirty = false;
            let down = window.get_mouse_down(MouseButton::Left);
            let click = down && !left_down;
            left_down = down;
            // Only the local overview interprets pointer actions. It never forwards input.
            let action = if overview_open && click {
                window.get_unscaled_mouse_pos(MouseMode::Discard)
                    .and_then(|point| overview::map_point(window.get_size(), point))
                    .and_then(|(x, y)| overview_panel.layout.hit(x, y))
            } else { None };
            if window.is_key_pressed(Key::Escape, KeyRepeat::No) {
                if overview_open {
                    overview_open = false;
                    overview_panel.clear();
                    presented = None;
                } else { break; }
            }
            if window.is_key_pressed(Key::Tab, KeyRepeat::No) {
                overview_open = !overview_open;
                overview_panel.clear();
                presented = None;
                overview_dirty = true;
            }
            if let Some(Action::Focus(index)) = action {
                policy.select(index);
                overview_open = false;
                overview_panel.clear();
                presented = None;
                show_selection(&mut window, &targets, &policy);
            }
            if action == Some(Action::Back) {
                overview_open = false;
                overview_panel.clear();
                presented = None;
            }
            for (index, key) in KEYS.iter().copied().take(targets.len()).enumerate() {
                if window.is_key_pressed(key, KeyRepeat::No) && policy.select(index) {
                    show_selection(&mut window, &targets, &policy);
                    overview_open = false;
                    overview_panel.clear();
                    presented = None;
                }
            }
            let selected = policy.selected();
            let toggle = match action {
                Some(Action::Toggle(index)) => Some(index),
                _ if window.is_key_pressed(Key::F9, KeyRepeat::No) => Some(selected),
                _ => None,
            };
            if let Some(index) = toggle {
                overview_panel.invalidate(index);
                if policy.is_active(index) {
                    policy.set_active(index, false);
                    if let Some(session) = &mut sessions[index] { session.stop(); }
                } else if sessions[index].is_none() {
                    policy.set_active(index, true);
                    sessions[index] = Some(start(&targets[index], endpoint.clone(), policy.priority(index).expect("Active slot")));
                } else { println!("Wait for the previous session to close before starting another."); }
                show_selection(&mut window, &targets, &policy);
                if index == selected { presented = None; }
                overview_dirty = true;
            }
            if action == Some(Action::Mode) || window.is_key_pressed(Key::F10, KeyRepeat::No) {
                policy.set_all_live(!policy.all_live());
                println!("All active devices at full authorized profile: {}", policy.all_live());
                overview_dirty = true;
            }
            if action == Some(Action::Fullscreen) || window.is_key_pressed(Key::F11, KeyRepeat::No) {
                // Recreate only the local window. Network sessions and decoded surfaces stay alive.
                match open_window(!fullscreen, monitor_id) {
                    Ok(next) => {
                        fullscreen = !fullscreen;
                        window = next;
                        show_selection(&mut window, &targets, &policy);
                        presented = None;
                        overview_dirty = true;
                    }
                    Err(error) => eprintln!("Cannot change fullscreen; keeping existing window: {error:#}"),
                }
            }
            if window.is_key_pressed(Key::F12, KeyRepeat::No) {
                if !fullscreen {
                    println!("Enter fullscreen with F11 before changing the local display.");
                } else {
                    match next_local_monitor(monitor_id).and_then(|next| {
                        next.map(|id| open_window(true, Some(id)).map(|window| (id, window))).transpose()
                    }) {
                        Ok(Some((id, next))) => {
                            monitor_id = Some(id);
                            window = next;
                            show_selection(&mut window, &targets, &policy);
                            presented = None;
                            overview_dirty = true;
                        }
                        Ok(None) => println!("No additional local display available."),
                        Err(error) => eprintln!("Cannot move fullscreen; keeping existing window: {error:#}"),
                    }
                }
            }
            update_priorities(&sessions, &policy);
            for (index, slot) in sessions.iter_mut().enumerate() {
                if slot.as_ref().is_some_and(|session| session.task.is_finished()) {
                    let session = slot.take().expect("Finished slot exists");
                    let result = session.task.await;
                    match result {
                        Ok(Ok(Some(stats))) => println!("{}: completed, {} changed / {} unchanged / {} no-update records", targets[index].name, stats.frames, stats.repeats, stats.no_updates),
                        Ok(Ok(None)) => println!("{}: disconnected locally", targets[index].name),
                        Ok(Err(error)) => eprintln!("{}: {error:#}", targets[index].name),
                        Err(error) => eprintln!("{}: task failed: {error}", targets[index].name),
                    }
                    policy.set_active(index, false);
                    overview_panel.invalidate(index);
                    overview_dirty = true;
                    if index == selected { presented = None; show_selection(&mut window, &targets, &policy); }
                }
            }
            // Sample local observations once per second, not every UI tick.
            if last_metrics_sample.elapsed() >= Duration::from_secs(1) {
                last_metrics_sample = Instant::now();
                for session in sessions.iter_mut().flatten() {
                    let _ = session.sampler.sample(session.started.elapsed(), session.counters.snapshot());
                }
                if !overview_open && policy.is_active(selected)
                    && let Some(rate) = sessions[selected].as_ref().and_then(|session| session.sampler.latest())
                {
                    let status = if sessions[selected].as_ref().is_some_and(|s| s.source_status.notice_due()) {
                        " | LAST FRAME / NO NEW SAMPLE"
                    } else { "" };
                    window.set_title(&format!("IntraLink | F{}: {}{} | RX {:.0} DEC {:.0} VIEW {:.0} FPS | {:.1} payload Mbps | TAB devices", selected + 1, targets[selected].name, status, rate.received_fps, rate.decoded_fps, rate.presented_fps, rate.payload_mbps));
                }
            }
            if overview_open {
                // Reuse the cached thumbnail if no new decoded sequence exists
                // or the decoder currently owns the lock. Never preserve pixels
                // after invalidation, disconnection or slot reuse.
                if overview_dirty || last_overview_paint.is_none_or(|last| last.elapsed() >= Duration::from_millis(100)) {
                    overview_panel.begin(policy.all_live(), fullscreen);
                    for (index, target) in targets.iter().enumerate() {
                        let previous = overview_panel.sequence(index);
                        let (snapshot, retain) = if policy.is_active(index) {
                            if let Some(session) = &sessions[index] {
                                match session.surface.try_lock() {
                                    Ok(surface) if !surface.is_invalidated() => (surface.snapshot_after(previous), previous > 0),
                                    Err(TryLockError::WouldBlock) => (None, true),
                                    _ => (None, false),
                                }
                            } else { (None, false) }
                        } else { (None, false) };
                        let state = if !policy.is_active(index) {
                            if sessions[index].is_some() { CardState::Closing } else { CardState::Inactive }
                        } else if let Some(ref image) = snapshot { CardState::Live(image) }
                        else if retain { CardState::Retain }
                        else { CardState::Waiting };
                        let rates = sessions[index].as_ref().and_then(|session| session.sampler.latest());
                        let notice = sessions[index].as_ref().is_some_and(|session| session.source_status.notice_due());
                        overview_panel.card_with_status(index, &target.name, selected == index, state, rates, notice);
                    }
                    last_overview_paint = Some(Instant::now());
                    window.update_with_buffer(overview_panel.pixels(), overview::WIDTH, overview::HEIGHT)
                        .map_err(|error| anyhow!("Workspace overview: {error}"))?;
                } else { window.update(); }
                tokio::time::sleep(Duration::from_secs_f64(1.0 / 60.0).saturating_sub(tick.elapsed())).await;
                continue;
            }
            last_overview_paint = None;
            let size = window.get_size();
            let notice = sessions[selected].as_ref().is_some_and(|session| session.source_status.notice_due());
            // Take an immutable reference under the short lock. The potentially
            // blocking native presentation happens after that lock is released.
            let snapshot = if policy.is_active(selected)
                && let Some(session) = &sessions[selected]
                && let Ok(surface) = session.surface.try_lock()
            {
                if surface.is_invalidated() { presented = None; }
                let previous = presented.filter(|(slot, _, old_size, old_notice)| *slot == selected && *old_size == size && *old_notice == notice)
                    .map_or(0, |(_, sequence, _, _)| sequence);
                surface.snapshot_after(previous)
            } else { None };
            if let Some(snapshot) = snapshot {
                let (width, height) = snapshot.dimensions();
                let presenting = Instant::now();
                let visible = if notice {
                    super::overview::last_frame_banner(snapshot.pixels(), (width, height), &mut banner_pixels)?;
                    &banner_pixels[..]
                } else { snapshot.pixels() };
                window.update_with_buffer(visible, width, height)
                    .map_err(|error| anyhow!("Workspace display: {error}"))?;
                if let Some(session) = &mut sessions[selected]
                    && snapshot.sequence() > session.last_presented_sequence
                {
                    session.counters.presented(presenting.elapsed());
                    session.last_presented_sequence = snapshot.sequence();
                }
                presented = Some((selected, snapshot.sequence(), size, notice));
            } else {
                if presented.is_none() {
                    window.update_with_buffer(&black, 2, 2).map_err(|error| anyhow!("Workspace display: {error}"))?;
                } else { window.update(); }
            }
            tokio::time::sleep(Duration::from_secs_f64(1.0 / 60.0).saturating_sub(tick.elapsed())).await;
        }
        Ok::<(), anyhow::Error>(())
    }.await;
    // Signal all stops before joining any task; one slow host cannot delay other stops.
    for session in sessions.iter_mut().flatten() {
        session.stop();
    }
    for session in sessions.into_iter().flatten() {
        match session.task.await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => eprintln!("Workspace shutdown: {error:#}"),
            Err(error) => eprintln!("Workspace shutdown task: {error}"),
        }
    }
    endpoint.close().await;
    drop(vault);
    result
}
