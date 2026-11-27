//! Explicit, foreground Windows controller pad. No global input hooks.
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use intralink_core::{
    input::{Button, Event, Focus, Key, MAX_SLOTS},
    input_layout::Layout,
};
use intralink_transport::{
    bind_workspace_controller,
    input::InputConnection,
    input_queue::{self, INTERACTIVE_BUDGET},
};
use intralink_vault::{Direction, Route, Vault};
use iroh::{Endpoint, EndpointId};
use minifb::{Key as LocalKey, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{MissedTickBehavior, sleep},
};

use crate::{
    input_host::private_lan,
    local::{approve, prompt_password},
    routing,
};

#[path = "input_keys.rs"]
mod keys;
#[path = "input_pad.rs"]
mod pad;

const POLL: Duration = Duration::from_millis(8);
const HEARTBEAT: Duration = Duration::from_millis(500);
const UI_LEASE: Duration = Duration::from_secs(1);

pub(super) fn window(
    title: &str,
    width: usize,
    height: usize,
    topmost: bool,
    color: u32,
) -> Result<Window> {
    let mut window = Window::new(
        title,
        width,
        height,
        WindowOptions {
            resize: false,
            topmost,
            ..WindowOptions::default()
        },
    )
    .map_err(|error| anyhow::anyhow!("Input window: {error}"))?;
    window.set_target_fps(0);
    window
        .update_with_buffer(&vec![color; width * height], width, height)
        .map_err(|error| anyhow::anyhow!("Input window: {error}"))?;
    Ok(window)
}

struct Slot {
    name: String,
    sender: input_queue::Sender,
    ready: Arc<AtomicBool>,
    last_ui: Arc<Mutex<Instant>>,
    task: JoinHandle<Result<()>>,
}

impl Slot {
    fn connected(&self) -> bool {
        self.ready.load(Ordering::Acquire) && !self.task.is_finished()
    }

    fn pulse(&self) -> Result<()> {
        *self
            .last_ui
            .lock()
            .map_err(|_| anyhow::anyhow!("Input UI lease poisoned"))? = Instant::now();
        Ok(())
    }

    fn queue(&self, event: Event, acknowledgment: Option<oneshot::Sender<()>>) -> Result<()> {
        ensure!(self.connected(), "Selected input host is not connected");
        self.sender.push(event, acknowledgment).map(|_| ())
    }

    async fn confirmed(&self, event: Event, window: &mut Window) -> Result<()> {
        let (acknowledgment, mut accepted) = oneshot::channel();
        self.pulse()?;
        self.queue(event, Some(acknowledgment))?;
        let deadline = tokio::time::Instant::now() + INTERACTIVE_BUDGET;
        loop {
            window.update();
            ensure!(
                window.is_open() && window.is_active() && !window.is_key_down(LocalKey::Escape),
                "Local input pad stopped or lost focus during switch"
            );
            self.pulse()?;
            tokio::select! {
                result = &mut accepted => return result.context("Input switch not acknowledged"),
                _ = tokio::time::sleep_until(deadline) => anyhow::bail!("Input switch deadline exceeded"),
                _ = sleep(POLL) => {}
            }
        }
    }
}

fn start_slot(endpoint: &Endpoint, name: String, identity: EndpointId, route: Route) -> Slot {
    let (sender, mut receiver) = input_queue::channel();
    let ready = Arc::new(AtomicBool::new(false));
    let worker_ready = ready.clone();
    let last_ui = Arc::new(Mutex::new(Instant::now()));
    let worker_ui = last_ui.clone();
    let endpoint = endpoint.clone();
    let task = tokio::spawn(async move {
        let result = async {
            let candidates: Vec<_> = routing::resolve(&route).await?.into_iter().filter(|addr| private_lan(*addr)).collect();
            ensure!(!candidates.is_empty(), "Input preview requires a private IPv4 route");
            let mut connection = InputConnection::connect(&endpoint, identity, &candidates).await?;
            worker_ready.store(true, Ordering::Release);
            let mut heartbeat = tokio::time::interval(HEARTBEAT);
            heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    biased;
                    command = receiver.recv() => {
                        let Some(command) = command? else { break; };
                        if connection.active() || command.event == Event::Activate {
                            verify_ui_lease(&worker_ui)?;
                        }
                        connection.send_with_budget(command.event, command.remaining()?).await?;
                        if let Some(acknowledgment) = command.acknowledgment {
                            // A canceled switch must not leave an active host.
                            acknowledgment.send(()).map_err(|_| anyhow::anyhow!("Input switch canceled"))?;
                        }
                        if command.event == Event::End { break; }
                    }
                    _ = heartbeat.tick() => {
                        if connection.active() {
                            verify_ui_lease(&worker_ui)?;
                            connection.send_with_budget(Event::Heartbeat, INTERACTIVE_BUDGET).await?;
                        } else {
                            connection.send(Event::Heartbeat).await?;
                        }
                    }
                }
            }
            Ok::<(), anyhow::Error>(())
        }.await;
        worker_ready.store(false, Ordering::Release);
        result
    });
    Slot {
        name,
        sender,
        ready,
        last_ui,
        task,
    }
}

fn verify_ui_lease(last_ui: &Mutex<Instant>) -> Result<()> {
    let elapsed = last_ui
        .lock()
        .map_err(|_| anyhow::anyhow!("Input UI lease poisoned"))?
        .elapsed();
    ensure!(
        elapsed <= UI_LEASE,
        "Input UI lease expired; closing active host"
    );
    Ok(())
}

const SELECT_KEYS: [LocalKey; MAX_SLOTS] = [
    LocalKey::F1,
    LocalKey::F2,
    LocalKey::F3,
    LocalKey::F4,
    LocalKey::F5,
    LocalKey::F6,
    LocalKey::F7,
    LocalKey::F8,
];

const BUTTONS: [(MouseButton, Button); 3] = [
    (MouseButton::Left, Button::Left),
    (MouseButton::Right, Button::Right),
    (MouseButton::Middle, Button::Middle),
];

#[derive(Default)]
struct Snapshot {
    keys: BTreeSet<Key>,
    buttons: BTreeSet<Button>,
    position: Option<(u16, u16)>,
}

fn local_keys(window: &Window) -> BTreeSet<Key> {
    window
        .get_keys()
        .into_iter()
        .filter_map(keys::key)
        .collect()
}
fn local_buttons(window: &Window) -> BTreeSet<Button> {
    BUTTONS
        .into_iter()
        .filter_map(|(local, remote)| window.get_mouse_down(local).then_some(remote))
        .collect()
}

fn normalized(window: &Window) -> Option<(u16, u16)> {
    let (x, y) = window.get_mouse_pos(MouseMode::Discard)?;
    let (width, height) = window.get_size();
    if !x.is_finite() || !y.is_finite() || width < 2 || height < 2 {
        return None;
    }
    let scale = |value: f32, extent: usize| {
        (value / (extent - 1) as f32 * 65535.0)
            .round()
            .clamp(0.0, 65535.0) as u16
    };
    Some((scale(x, width), scale(y, height)))
}

impl Snapshot {
    fn forward(&mut self, window: &Window, slot: &Slot, position: (u16, u16)) -> Result<()> {
        slot.pulse()?;
        if self.position != Some(position) {
            slot.queue(
                Event::Move {
                    x: position.0,
                    y: position.1,
                },
                None,
            )?;
            self.position = Some(position);
        }
        let current = local_keys(window);
        // Release nonmodifiers before modifiers; press modifiers before
        // nonmodifiers so a chord has deterministic destination order.
        for key in self.keys.difference(&current).copied() {
            slot.queue(Event::Key { key, down: false }, None)?;
        }
        let mut down: Vec<_> = current.difference(&self.keys).copied().collect();
        down.sort_by_key(|key| (key.usage() < 224, key.usage()));
        for key in down {
            slot.queue(Event::Key { key, down: true }, None)?;
        }
        self.keys = current;
        let buttons = local_buttons(window);
        for button in self.buttons.difference(&buttons).copied() {
            slot.queue(
                Event::Button {
                    button,
                    down: false,
                },
                None,
            )?;
        }
        for button in buttons.difference(&self.buttons).copied() {
            slot.queue(Event::Button { button, down: true }, None)?;
        }
        self.buttons = buttons;
        if let Some((x, y)) = window.get_scroll_wheel() {
            for (horizontal, value) in [(true, x), (false, y)] {
                if value.is_finite() {
                    let steps = value.round().clamp(-8.0, 8.0) as i16;
                    if steps != 0 {
                        slot.queue(Event::Wheel { horizontal, steps }, None)?;
                    }
                }
            }
        }
        Ok(())
    }
}

async fn switch_focus(
    focus: &mut Focus,
    target: Option<usize>,
    slots: &[Slot],
    window: &mut Window,
) -> Result<()> {
    if focus.active() == target {
        return Ok(());
    }
    if let Some(target) = target {
        ensure!(
            slots.get(target).is_some_and(Slot::connected),
            "Target input host unavailable"
        );
    }
    if let Some(previous) = focus.begin(target)? {
        slots[previous].confirmed(Event::Deactivate, window).await?;
    }
    let next = focus.released()?;
    if let Some(next) = next {
        ensure!(
            window.is_active()
                && normalized(window).is_some()
                && local_keys(window).is_empty()
                && local_buttons(window).is_empty(),
            "Release local keys/buttons and keep the pointer in the pad before activating"
        );
        slots[next].confirmed(Event::Activate, window).await?;
    }
    focus.activated(next)?;
    Ok(())
}

struct Spatial {
    layout: Layout,
    armed: bool,
    hover: Option<(usize, Instant)>,
    canvas: Vec<u32>,
    painted: Option<(u8, Option<usize>, bool)>,
}

impl Spatial {
    fn new(slots: usize) -> Result<Self> {
        Ok(Self {
            layout: Layout::new(slots).context("Invalid spatial input layout")?,
            armed: false,
            hover: None,
            canvas: vec![0; pad::WIDTH * pad::HEIGHT],
            painted: None,
        })
    }

    async fn step(
        &mut self,
        focus: &mut Focus,
        previous: &mut Snapshot,
        slots: &[Slot],
        window: &mut Window,
    ) -> Result<()> {
        if !window.is_active() {
            self.armed = false;
            self.hover = None;
            return Ok(());
        }
        let toggle = window.is_key_pressed(LocalKey::F9, KeyRepeat::No);
        let disarm = window.is_key_pressed(LocalKey::F12, KeyRepeat::No) || (toggle && self.armed);
        let zone = pad::pointer_zone(window, self.layout);
        if disarm || zone.is_none() {
            switch_focus(focus, None, slots, window).await?;
            *previous = Snapshot::default();
            self.armed = false;
            self.hover = None;
            return Ok(());
        }
        if toggle && local_keys(window).is_empty() && local_buttons(window).is_empty() {
            self.armed = true;
            self.hover = None;
        }
        if !self.armed {
            return Ok(());
        }
        let zone = zone.context("Missing spatial pointer")?;
        let target = zone.target.filter(|index| slots[*index].connected());
        if focus.active() == target {
            self.hover = None;
            if let Some(index) = target {
                previous.forward(window, &slots[index], (zone.x, zone.y))?;
            }
            return Ok(());
        }
        // Crossing never carries a drag or chord to another machine. Stop
        // forwarding immediately, acknowledge old release, then dwell.
        let held = !previous.keys.is_empty()
            || !previous.buttons.is_empty()
            || !local_keys(window).is_empty()
            || !local_buttons(window).is_empty();
        if focus.active().is_some() {
            switch_focus(focus, None, slots, window).await?;
            *previous = Snapshot::default();
            self.hover = None;
            if held {
                self.armed = false;
                return Ok(());
            }
        }
        let Some(index) = target else {
            self.hover = None;
            return Ok(());
        };
        if held {
            self.hover = None;
            return Ok(());
        }
        match self.hover {
            Some((candidate, since)) if candidate == index => {
                if since.elapsed() >= Duration::from_millis(80) {
                    switch_focus(focus, Some(index), slots, window).await?;
                    *previous = Snapshot::default();
                    self.hover = None;
                    // A fresh pointer zone is required on the next UI turn.
                }
            }
            _ => self.hover = Some((index, Instant::now())),
        }
        Ok(())
    }

    fn paint(&mut self, slots: &[Slot], focus: &Focus, window: &mut Window) -> Result<()> {
        let ready = slots.iter().enumerate().fold(0u8, |mask, (index, slot)| {
            if slot.connected() {
                mask | (1 << index)
            } else {
                mask
            }
        });
        let state = (ready, focus.active(), self.armed);
        if self.painted != Some(state) {
            pad::render(
                self.layout,
                &mut self.canvas,
                ready,
                focus.active(),
                self.armed,
            );
            window
                .update_with_buffer(&self.canvas, pad::WIDTH, pad::HEIGHT)
                .map_err(|error| anyhow::anyhow!("Spatial input pad: {error}"))?;
            self.painted = Some(state);
        }
        Ok(())
    }
}

fn title(slots: &[Slot], focus: &Focus, spatial: Option<&Spatial>) -> String {
    let target = focus
        .active()
        .map_or("LOCAL", |index| slots[index].name.as_str());
    let connected = slots.iter().filter(|slot| slot.connected()).count();
    let controls = match spatial {
        Some(pad) if pad.armed => "SPATIAL ARMED | 0 local, F12 disarm, ESC close",
        Some(_) => "SPATIAL SAFE | F9 arm, ESC close",
        None => "F1-F8 select, F12 local, ESC close",
    };
    format!(
        "IntraLink INPUT | {target} | {connected}/{} connected | {controls}",
        slots.len()
    )
}

pub(super) async fn workspace(path: &Path, names: &[String], spatial: bool) -> Result<()> {
    ensure!(
        (1..=MAX_SLOTS).contains(&names.len()),
        "Select 1-8 distinct input hosts"
    );
    let vault = Vault::lock(path)?;
    let password = prompt_password()?;
    let profile = vault.load(password.as_bytes())?;
    drop(password);
    ensure!(
        profile.role().can_initiate(),
        "Profile cannot initiate input"
    );
    let mut identities = BTreeSet::new();
    let mut targets = Vec::new();
    for name in names {
        let peer = profile.peer(name, Direction::Outbound)?;
        let identity = peer.identity()?;
        ensure!(identities.insert(identity), "Duplicate input host identity");
        targets.push((
            name.clone(),
            identity,
            peer.route().context("Outbound route missing")?.clone(),
        ));
    }
    let secret = profile.secret_key();
    drop(profile);
    approve(if spatial {
        "Open the visible spatial input pad? F9 explicitly arms automatic switching between numbered zones; F12 disarms and releases input. No global hook or screen stream."
    } else {
        "Open the visible input-only pad for these paired hosts? F1-F8 explicitly activate a ready host; F12 returns to local. No global hook, screen stream or unattended control."
    })?;
    let mut window = window(
        "IntraLink INPUT | LOCAL | connecting",
        960,
        540,
        false,
        0x00212c3b,
    )?;
    let endpoint = bind_workspace_controller(secret).await?;
    let mut slots: Vec<_> = targets
        .into_iter()
        .map(|(name, identity, route)| start_slot(&endpoint, name, identity, route))
        .collect();
    for (index, slot) in slots.iter().enumerate() {
        println!("F{}: {}", index + 1, slot.name);
    }
    println!(
        "Focus the input pad. Its pointer maps to the active host's primary display. F12 releases the selected host; Escape closes. Do not use two profiles on the same computer."
    );
    let mut focus = Focus::default();
    let mut previous = Snapshot::default();
    let mut last_title = String::new();
    let mut spatial = spatial.then(|| Spatial::new(slots.len())).transpose()?;
    if spatial.is_some() {
        println!(
            "Spatial pad: zone 0 is LOCAL; zones 1-8 follow the printed device order. Press F9 to arm; cross inside the pad with keys/buttons released. F12 disarms."
        );
    }
    let local = async {
        loop {
            window.update();
            if !window.is_open() || window.is_key_down(LocalKey::Escape) {
                break;
            }
            if focus.active().is_some() && !window.is_active() {
                anyhow::bail!(
                    "Input pad lost focus; disconnecting rather than forwarding stale input"
                );
            }
            if let Some(spatial) = &mut spatial {
                spatial
                    .step(&mut focus, &mut previous, &slots, &mut window)
                    .await?;
                spatial.paint(&slots, &focus, &mut window)?;
            } else if window.is_active() {
                let select = window
                    .get_keys_pressed(KeyRepeat::No)
                    .into_iter()
                    .find_map(|key| SELECT_KEYS.iter().position(|candidate| *candidate == key));
                let return_local = window.is_key_pressed(LocalKey::F12, KeyRepeat::No)
                    || (focus.active().is_some() && normalized(&window).is_none());
                if return_local {
                    switch_focus(&mut focus, None, &slots, &mut window).await?;
                    previous = Snapshot::default();
                } else if let Some(index) = select {
                    if slots.get(index).is_some_and(Slot::connected)
                        && local_keys(&window).is_empty()
                        && local_buttons(&window).is_empty()
                        && normalized(&window).is_some()
                    {
                        if focus.active() != Some(index) {
                            switch_focus(&mut focus, Some(index), &slots, &mut window).await?;
                            previous = Snapshot::default();
                        }
                    } else {
                        println!(
                            "Target unavailable or local input still held; select again when ready."
                        );
                    }
                }
                if let Some(index) = focus.active() {
                    ensure!(window.is_active(), "Input pad lost focus");
                    let position = normalized(&window).context("Pointer left the input pad")?;
                    previous.forward(&window, &slots[index], position)?;
                }
            }
            let next = title(&slots, &focus, spatial.as_ref());
            if next != last_title {
                window.set_title(&next);
                last_title = next;
            }
            sleep(POLL).await;
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    // Abort every owned worker on close/focus loss. Dropping the connection
    // closes its QUIC session, causing the host to release its owned input.
    focus.cancel();
    for slot in &slots {
        slot.task.abort();
    }
    for slot in &mut slots {
        let _ = (&mut slot.task).await;
    }
    endpoint.close().await;
    local
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_worker_requires_recent_ui_lease_for_active_session() {
        let current = Mutex::new(Instant::now());
        assert!(verify_ui_lease(&current).is_ok());
        let expired = Mutex::new(Instant::now() - UI_LEASE - Duration::from_secs(1));
        assert!(verify_ui_lease(&expired).is_err());
    }
}
