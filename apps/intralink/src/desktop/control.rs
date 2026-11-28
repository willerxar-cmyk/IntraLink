//! One foreground window, two independent explicitly approved capabilities.
//! All native input is window-scoped. Video decode and input ACKs use the
//! existing independent workers, transports, identities and leases.
#[path = "control_canvas.rs"]
mod canvas;
#[path = "control_gate.rs"]
mod gate;
use self::{
    canvas::{Action, Canvas, Chrome, Click, InputState, Lane},
    gate::{Decision, Gate, TargetRegion},
};
use super::{
    device_picker::{self, Kind},
    workspace::{self, Session, Target},
};
use crate::{
    input_slots::{Slot, input_route, start_slot},
    input_ui::{Snapshot, desktop_key, local_buttons},
    local::prompt_password,
};
use anyhow::{Context, Result, ensure};
use intralink_core::{
    desktop::ViewPriority,
    display_geometry::Display,
    input::{Event, Key as RemoteKey},
};
use intralink_transport::{
    bind_workspace_controller, desktop::ViewMode, input_queue::INTERACTIVE_BUDGET,
};
use intralink_vault::{Direction, Route, SessionKind, SessionVault};
use iroh::{Endpoint, SecretKey};
use minifb::{Key, MouseButton, MouseMode, ScaleMode, Window, WindowOptions};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::TryLockError,
    time::{Duration, Instant},
};
use tokio::{
    sync::oneshot,
    time::{sleep, timeout},
};

const TICK: Duration = Duration::from_millis(8);
const PRESENT: Duration = Duration::from_millis(16);

struct ScreenLane {
    endpoint: Option<Endpoint>,
    worker: Option<Session>,
    status: Lane,
}
impl ScreenLane {
    fn new() -> Self {
        Self {
            endpoint: None,
            worker: None,
            status: Lane::Idle,
        }
    }
    async fn start(&mut self, target: &Target, secret: &SecretKey) -> Result<()> {
        ensure!(
            self.status.can_start() && self.worker.is_none() && self.endpoint.is_none(),
            "Screen lane already owns a session"
        );
        match bind_workspace_controller(secret.clone()).await {
            Ok(endpoint) => {
                self.worker = Some(workspace::start(
                    target,
                    endpoint.clone(),
                    ViewPriority::Foreground,
                ));
                self.endpoint = Some(endpoint);
                self.status = Lane::Connecting;
            }
            Err(e) => {
                eprintln!("Screen connection: {e:#}");
                self.status = Lane::Failed;
            }
        }
        Ok(())
    }
    fn stop(&mut self) {
        if let Some(worker) = &mut self.worker {
            worker.stop();
            self.status = Lane::Stopping;
        }
    }
    async fn reap(&mut self) {
        if self.worker.as_ref().is_some_and(|w| w.task.is_finished()) {
            let worker = self.worker.take().expect("Finished screen worker");
            let result = worker.task.await;
            self.status = match result {
                Ok(Ok(_)) => Lane::Ended,
                Ok(Err(e)) => {
                    eprintln!("Screen session: {e:#}");
                    Lane::Failed
                }
                Err(e) => {
                    eprintln!("Screen worker: {e}");
                    Lane::Failed
                }
            };
            if let Some(endpoint) = self.endpoint.take() {
                endpoint.close().await;
            }
        }
    }
    async fn close(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take()
            && let Err(e) = worker.task.await.context("Screen join").and_then(|r| r)
        {
            eprintln!("Screen cleanup: {e:#}");
        }
        if let Some(endpoint) = self.endpoint.take() {
            endpoint.close().await;
        }
        self.status = Lane::Ended;
    }
    fn stale(&self) -> bool {
        self.status == Lane::Live
            && self
                .worker
                .as_ref()
                .is_some_and(|w| w.source_status.notice_due() || !w.source_status.sample_recent())
    }
    fn live(&self) -> bool {
        self.status == Lane::Live && self.worker.as_ref().is_some_and(|w| !w.task.is_finished())
    }
}

struct InputLane {
    endpoint: Option<Endpoint>,
    slot: Option<Slot>,
    status: Lane,
}
impl InputLane {
    fn new() -> Self {
        Self {
            endpoint: None,
            slot: None,
            status: Lane::Idle,
        }
    }
    async fn start(&mut self, target: &Target, route: &Route, secret: &SecretKey) -> Result<()> {
        ensure!(
            self.status.can_start() && self.slot.is_none() && self.endpoint.is_none(),
            "Input lane already owns a session"
        );
        match bind_workspace_controller(secret.clone()).await {
            Ok(endpoint) => {
                self.slot = Some(start_slot(
                    &endpoint,
                    target.name.clone(),
                    target.identity,
                    route.clone(),
                    vec![],
                    true,
                ));
                self.endpoint = Some(endpoint);
                self.status = Lane::Connecting;
            }
            Err(e) => {
                eprintln!("Input connection: {e:#}");
                self.status = Lane::Failed;
            }
        }
        Ok(())
    }
    fn display(&self) -> Option<Display> {
        self.slot.as_ref().and_then(Slot::display)
    }
    async fn reap(&mut self) {
        if self.slot.as_ref().is_some_and(|s| s.task.is_finished()) {
            let slot = self.slot.take().expect("Finished input worker");
            self.status = match slot.task.await {
                Ok(Ok(())) => Lane::Ended,
                Ok(Err(e)) => {
                    eprintln!("Input session: {e:#}");
                    Lane::Failed
                }
                Err(e) => {
                    eprintln!("Input worker: {e}");
                    Lane::Failed
                }
            };
            if let Some(endpoint) = self.endpoint.take() {
                endpoint.close().await;
            }
        } else if self.display().is_some() {
            self.status = Lane::Live;
        }
    }
    fn cancel(&mut self) {
        if let Some(slot) = &self.slot {
            slot.task.abort();
            self.status = Lane::Stopping;
        }
    }
    async fn end(&mut self, window: &mut Window) {
        if let Some(slot) = self.slot.as_ref().filter(|slot| slot.connected())
            && let Err(e) = release(slot, window, Event::End).await
        {
            eprintln!("Input End not acknowledged; disconnecting: {e:#}");
        }
        self.close(false).await;
    }
    async fn close(&mut self, failed: bool) {
        self.cancel();
        if let Some(slot) = self.slot.take() {
            let _ = slot.task.await;
        }
        if let Some(endpoint) = self.endpoint.take() {
            endpoint.close().await;
        }
        self.status = if failed { Lane::Failed } else { Lane::Ended };
    }
}

fn pointer(window: &Window) -> Option<(usize, usize)> {
    canvas::map_window(
        window.get_size(),
        window.get_unscaled_mouse_pos(MouseMode::Discard)?,
    )
}
fn stop_key(window: &Window) -> bool {
    window.is_key_down(Key::F12) || window.is_key_down(Key::Escape)
}
fn neutral(window: &Window) -> bool {
    window.get_keys().is_empty()
        && local_buttons(window).is_empty()
        && window
            .get_scroll_wheel()
            .is_none_or(|(x, y)| x == 0.0 && y == 0.0)
}
fn keys(window: &Window) -> Result<BTreeSet<RemoteKey>> {
    window
        .get_keys()
        .into_iter()
        .map(|k| desktop_key(k).context("Unsupported or reserved key; control released"))
        .collect()
}

/// A release is safe even when focus is already lost. Unlike activation,
/// it never consumes UI input or requires a focused pointer. If its bounded
/// ACK fails, the caller aborts only the input worker and closes its endpoint.
async fn release(slot: &Slot, window: &mut Window, event: Event) -> Result<()> {
    let (tx, mut rx) = oneshot::channel();
    slot.pulse()?;
    ensure!(
        matches!(event, Event::Deactivate | Event::End),
        "Invalid local stop action"
    );
    slot.queue(event, Some(tx))?;
    timeout(INTERACTIVE_BUDGET, async {
        loop {
            tokio::select! {
                result=&mut rx=>return result.context("Input release not acknowledged"),
                _=sleep(TICK)=>{if window.is_open(){window.update();}slot.pulse()?;}
            }
        }
    })
    .await
    .context("Input release deadline exceeded")??;
    Ok(())
}

struct Viewer {
    canvas: Canvas,
    screen: ScreenLane,
    input: InputLane,
    gate: Gate,
    snapshot: Snapshot,
    click: Click,
    frame: Option<(usize, usize)>,
    sequence: usize,
    verified: bool,
    // A second explicit click confirms monitor matching. No source identity
    // attestation is implied by input/2 metadata or by the video pixels.
    verify_pending: bool,
    last_chrome: Option<Chrome>,
    last_placeholder: Option<(Lane, Option<Display>)>,
    painted_region: Option<TargetRegion>,
    last_present: Instant,
}
impl Viewer {
    fn new() -> Self {
        Self {
            canvas: Canvas::new(),
            screen: ScreenLane::new(),
            input: InputLane::new(),
            gate: Gate::default(),
            snapshot: Snapshot::default(),
            click: Click::default(),
            frame: None,
            sequence: 0,
            verified: false,
            verify_pending: false,
            last_chrome: None,
            last_placeholder: None,
            painted_region: None,
            last_present: Instant::now(),
        }
    }
    fn clear_match(&mut self) {
        self.verified = false;
        self.verify_pending = false;
    }
    fn clear_frame(&mut self) {
        self.frame = None;
        self.sequence = 0;
        self.clear_match();
        self.last_placeholder = None;
        self.painted_region = None;
    }
    fn desired_region(&self, window: &Window) -> Option<TargetRegion> {
        let display = self.input.display()?;
        let video = if self.screen.live() && self.verified && !self.screen.stale() {
            Some(self.frame?)
        } else if self.screen.status.can_start() {
            None
        } else {
            return None;
        };
        let rect = match video {
            Some(size) => canvas::video_region(size, display)?,
            None => canvas::input_region(display)?,
        };
        Some(TargetRegion {
            rect,
            display,
            video,
            window: window.get_size(),
        })
    }
    fn region(&self, window: &Window) -> Option<TargetRegion> {
        self.desired_region(window)
            .filter(|region| self.painted_region == Some(*region))
    }
    fn chrome(&self, window: &Window) -> Chrome {
        Chrome {
            screen: self.screen.status,
            input: self.input.status,
            state: self.gate.state(),
            verified: self.verified,
            verify_pending: self.verify_pending,
            verify_allowed: self.screen.live()
                && self.frame.is_some()
                && !self.screen.stale()
                && self.input.display().is_some(),
            arm_allowed: self.desired_region(window).is_some(),
            stale: self.screen.stale(),
            display: self.input.display(),
        }
    }
    async fn disarm(&mut self, window: &mut Window) {
        let active = self.gate.state() == InputState::Active;
        self.gate.clear();
        self.snapshot = Snapshot::default();
        self.click = Click::default();
        if active
            && let Some(slot) = &self.input.slot
            && let Err(e) = release(slot, window, Event::Deactivate).await
        {
            eprintln!("Release failed; ending input only: {e:#}");
            self.input.close(true).await;
        }
    }
    async fn activate(&mut self, window: &mut Window, target: TargetRegion) -> Result<()> {
        let slot = self
            .input
            .slot
            .as_ref()
            .context("Input listener not ready")?;
        ensure!(
            neutral(window) && intralink_input_win::neutral_for_handoff(),
            "Release local keys/buttons before control"
        );
        let screen = &self.screen;
        let guard = |w: &Window| {
            !stop_key(w)
                && neutral(w)
                && intralink_input_win::neutral_for_handoff()
                && w.get_size() == target.window
                && pointer(w)
                    .and_then(|p| canvas::normalized(target.rect, p))
                    .is_some()
                && slot.display() == Some(target.display)
                && (target.video.is_none() || screen.live() && !screen.stale())
        };
        slot.confirmed_guarded(Event::Activate, window, guard)
            .await?;
        // A moved pointer must use the current reviewed content position,
        // not an obsolete position from the first ACK wait.
        let entry = pointer(window)
            .and_then(|p| canvas::normalized(target.rect, p))
            .context("Pointer left control region")?;
        slot.confirmed_guarded(
            Event::Move {
                x: entry.0,
                y: entry.1,
            },
            window,
            guard,
        )
        .await?;
        ensure!(
            self.gate.acknowledge(target),
            "Control arm changed before acknowledgment"
        );
        self.snapshot = Snapshot::default();
        Ok(())
    }
    async fn handle(
        &mut self,
        action: Action,
        window: &mut Window,
        target: &Target,
        route: &Route,
        secret: &SecretKey,
    ) -> Result<bool> {
        if action != Action::Verify {
            self.verify_pending = false;
        }
        match action {
            Action::Close => return Ok(false),
            Action::Screen => {
                self.disarm(window).await;
                self.clear_frame();
                if self.screen.status.open() {
                    self.screen.stop();
                } else if self.screen.status.can_start() {
                    self.screen.start(target, secret).await?;
                }
            }
            Action::Input => {
                self.disarm(window).await;
                self.clear_match();
                if self.input.status.open() {
                    self.input.end(window).await;
                } else if self.input.status.can_start() {
                    self.input.start(target, route, secret).await?;
                }
            }
            Action::Verify => {
                if self.verified {
                    self.clear_match();
                } else if self.verify_pending {
                    self.verified = true;
                    self.verify_pending = false;
                } else {
                    self.verify_pending = true;
                }
            }
            Action::Arm => {
                if self.gate.state() != InputState::Safe {
                    self.disarm(window).await;
                } else if let Some(region) = self.region(window)
                    && neutral(window)
                {
                    self.gate.arm(region);
                }
            }
        }
        Ok(true)
    }

    fn refresh_media(&mut self) -> Result<bool> {
        if !self.screen.status.open() {
            if self.frame.is_some() {
                self.clear_frame();
            }
            return Ok(false);
        }
        let Some(worker) = self.screen.worker.as_ref() else {
            return Ok(false);
        };
        let (invalid, frame) = match worker.surface.try_lock() {
            Ok(surface) => (
                surface.is_invalidated(),
                surface.snapshot_after(self.sequence),
            ),
            Err(TryLockError::WouldBlock) => (false, None),
            Err(TryLockError::Poisoned(_)) => (true, None),
        };
        if invalid {
            self.screen.stop();
            self.clear_frame();
            return Ok(false);
        }
        let Some(frame) = frame else {
            return Ok(false);
        };
        let geometry = frame.dimensions();
        if self.frame != Some(geometry) {
            self.clear_match();
        }
        self.canvas.frame(&frame)?;
        self.frame = Some(geometry);
        self.sequence = frame.sequence();
        self.screen.status = Lane::Live;
        self.last_placeholder = None;
        Ok(true)
    }

    async fn tick(
        &mut self,
        window: &mut Window,
        target: &Target,
        route: &Route,
        secret: &SecretKey,
    ) -> Result<bool> {
        if !window.is_open() {
            return Ok(false);
        }
        // A finished/failed video worker can no longer support direct
        // image control. Release input before awaiting screen cleanup.
        if self
            .screen
            .worker
            .as_ref()
            .is_some_and(|w| w.task.is_finished())
        {
            self.disarm(window).await;
            self.clear_frame();
        }
        self.input.reap().await;
        self.screen.reap().await;
        if self.input.display().is_none() {
            self.gate.clear();
            self.snapshot = Snapshot::default();
            self.clear_match();
        }
        if !self.screen.status.open() && self.frame.is_some() {
            self.clear_frame();
        }
        if self.screen.stale() {
            self.clear_match();
        }
        if !window.is_active() {
            self.verify_pending = false;
        }

        let before = self.region(window);
        let new_frame = if self.last_present.elapsed() >= PRESENT {
            self.refresh_media()?
        } else {
            false
        };
        // Never change the coordinate mapping under an active or pending
        // control. Release first, then show the changed frame/placeholder.
        if self.gate.state() != InputState::Safe
            && (before.is_none() || self.desired_region(window) != before)
        {
            self.disarm(window).await;
        }
        let state = self.chrome(window);
        let mut dirty = new_frame
            || self
                .painted_region
                .is_some_and(|p| p.window != window.get_size());
        if self.last_chrome != Some(state) {
            self.click = Click::default();
            self.canvas.chrome(&target.name, state);
            self.last_chrome = Some(state);
            dirty = true;
        }
        if self.frame.is_none() {
            let key = (self.screen.status, self.input.display());
            if self.last_placeholder != Some(key) {
                self.canvas.placeholder(key.0, key.1);
                self.last_placeholder = Some(key);
                dirty = true;
            }
        }
        // Exactly one normal event pump per tick. ACK waits intentionally
        // pump only to validate neutral state and stop/focus guards.
        if dirty {
            window
                .update_with_buffer(&self.canvas.pixels, canvas::WIDTH, canvas::HEIGHT)
                .map_err(|e| anyhow::anyhow!("Unified viewer: {e}"))?;
            self.last_present = Instant::now();
            self.painted_region = self.desired_region(window);
        } else {
            window.update();
        }
        if !window.is_open() || window.is_key_down(Key::Escape) {
            return Ok(false);
        }
        if !window.is_active() {
            self.verify_pending = false;
        }
        let region = self.region(window);
        let decision = self.gate.decide(
            region,
            pointer(window),
            window.is_active(),
            neutral(window),
            stop_key(window),
        );
        match decision {
            Decision::Release => self.disarm(window).await,
            Decision::Activate(_) => {
                let region = region.context("Missing armed region")?;
                if let Err(e) = self.activate(window, region).await {
                    eprintln!("Activation failed: {e:#}");
                    self.gate.clear();
                    self.snapshot = Snapshot::default();
                    self.input.close(true).await;
                }
            }
            Decision::Forward(position) => {
                let slot = self
                    .input
                    .slot
                    .as_ref()
                    .context("Missing active input worker")?;
                match keys(window) {
                    Ok(current) => {
                        if let Err(e) = self
                            .snapshot
                            .forward_with_keys(window, slot, position, current)
                        {
                            eprintln!("Input forwarding failed: {e:#}");
                            self.gate.clear();
                            self.snapshot = Snapshot::default();
                            self.input.close(true).await;
                        }
                    }
                    Err(e) => {
                        eprintln!("{e:#}");
                        self.disarm(window).await;
                    }
                }
            }
            Decision::Idle => {}
        }
        if self.gate.state() != InputState::Active {
            let chrome = self.chrome(window);
            // Only interact with the controls that were actually displayed.
            let left = window.get_mouse_down(MouseButton::Left);
            let local = window.is_active()
                && !stop_key(window)
                && window.get_keys().is_empty()
                && !window.get_mouse_down(MouseButton::Right)
                && !window.get_mouse_down(MouseButton::Middle);
            let displayed = self.last_chrome == Some(chrome);
            if let Some(action) =
                self.click
                    .step(local && displayed, left, chrome.hit(pointer(window)))
                && !self.handle(action, window, target, route, secret).await?
            {
                return Ok(false);
            }
        } else {
            self.click = Click::default();
        }
        Ok(true)
    }
}

pub(crate) async fn run(
    path: &Path,
    device: Option<String>,
    choose: bool,
    input_port: u16,
    mode: ViewMode,
) -> Result<()> {
    super::require_graphical_terminal()?;
    ensure!(input_port != 0, "Input port must be nonzero");
    // Fixed nonblocking order. These leases also serialize the legacy
    // separate desktop commands and forbid any profile mutation.
    let screen_vault = SessionVault::lock(path, SessionKind::Screen)?;
    let _input_vault = SessionVault::lock(path, SessionKind::Input)?;
    let password = prompt_password()?;
    let profile = screen_vault.load(password.as_bytes())?;
    drop(password);
    ensure!(
        profile.role().can_initiate(),
        "Profile cannot initiate sessions"
    );
    let name = if choose {
        let (names, _) = device_picker::choose(&profile, Kind::Control).await?;
        ensure!(names.len() == 1, "Select exactly one CLIENT");
        names
            .into_iter()
            .next()
            .context("Missing selected CLIENT")?
    } else {
        device.context("Select a paired CLIENT")?
    };
    let peer = profile.peer(&name, Direction::Outbound)?;
    let route = peer.route().context("Missing saved screen route")?.clone();
    ensure!(
        route.port() != input_port,
        "Use a distinct input/2 listener port"
    );
    let input_route = input_route(&route, Some(input_port))?;
    let target = Target {
        name,
        identity: peer.identity()?,
        route,
        mode,
    };
    let secret = profile.secret_key();
    drop(profile);
    println!(
        "CLIENT: {} ({})\nScreen route: {}:{}\nInput/2 route: {}:{}\nThe window starts SAFE; click each capability separately. The CLIENT must separately approve both. For direct image control, independently verify that the shared monitor is the input/2 primary display. F12 releases; Escape closes.",
        target.name,
        target.identity,
        target.route.host(),
        target.route.port(),
        input_route.host(),
        input_route.port()
    );
    let mut window = Window::new(
        "IntraLink | Screen + Input / SAFE",
        1280,
        720,
        WindowOptions {
            resize: true,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| anyhow::anyhow!("Unified control window: {e}"))?;
    window.set_target_fps(0);
    let mut viewer = Viewer::new();
    // Paint and establish a fresh visible UI before any listener connection.
    let chrome = viewer.chrome(&window);
    viewer.canvas.chrome(&target.name, chrome);
    viewer.canvas.placeholder(Lane::Idle, None);
    window
        .update_with_buffer(&viewer.canvas.pixels, canvas::WIDTH, canvas::HEIGHT)
        .map_err(|e| anyhow::anyhow!("Unified window: {e}"))?;
    viewer.last_chrome = Some(chrome);
    let result = async {
        loop {
            if !viewer
                .tick(&mut window, &target, &input_route, &secret)
                .await?
            {
                break;
            }
            sleep(TICK).await;
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    // Cancel both producers before waiting on either cleanup. In particular
    // input must not stay active while video decoding or shutdown stalls.
    viewer.gate.clear();
    viewer.screen.stop();
    viewer.input.end(&mut window).await;
    viewer.screen.close().await;
    result
}

#[cfg(test)]
#[path = "control_integration_tests.rs"]
mod tests;
