//! Explicit, foreground Windows controller pad. No global input hooks.
use std::{
    collections::BTreeSet,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use intralink_core::{
    input::{Button, Event, Focus, Key, MAX_SLOTS},
    input_layout::Layout,
};
use intralink_transport::{bind_workspace_controller, input_queue::INTERACTIVE_BUDGET};
use minifb::{Key as LocalKey, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use tokio::{sync::oneshot, time::sleep};

use crate::{
    input_slots::{Slot, load_targets, start_slot},
    local::approve,
};

#[path = "input_keys.rs"]
mod keys;
#[path = "input_pad.rs"]
mod pad;
#[path = "input_portal.rs"]
mod portal;

const POLL: Duration = Duration::from_millis(8);

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

impl Slot {
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
    drag: Option<usize>,
    was_down: bool,
    last_cell: Option<usize>,
    effect: Option<((usize, usize), Instant, bool)>,
    portal_mode: portal::Mode,
    portal_patch: portal::Patch,
}

impl Spatial {
    fn new(slots: usize) -> Result<Self> {
        Ok(Self {
            layout: Layout::arranged(slots).context("Invalid spatial input layout")?,
            armed: false,
            hover: None,
            canvas: vec![0; pad::WIDTH * pad::HEIGHT],
            painted: None,
            drag: None,
            was_down: false,
            last_cell: None,
            effect: None,
            portal_mode: portal::Mode::default(),
            portal_patch: portal::Patch::default(),
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
            self.drag = None;
            self.was_down = false;
            self.last_cell = None;
            self.effect = None;
            return Ok(());
        }
        let toggle = window.is_key_pressed(LocalKey::F9, KeyRepeat::No);
        let disarm = window.is_key_pressed(LocalKey::F12, KeyRepeat::No) || (toggle && self.armed);
        let zone = pad::pointer_zone(window, self.layout);
        let point = pad::pointer(window);
        let cell = point.and_then(|(x, y)| self.layout.cell_at(x, y, pad::WIDTH, pad::HEIGHT));
        if disarm || zone.is_none() {
            switch_focus(focus, None, slots, window).await?;
            *previous = Snapshot::default();
            self.armed = false;
            self.hover = None;
            self.drag = None;
            self.was_down = window.get_mouse_down(MouseButton::Left);
            self.last_cell = None;
            self.effect = None;
            return Ok(());
        }
        if toggle && local_keys(window).is_empty() && local_buttons(window).is_empty() {
            self.armed = true;
            self.hover = None;
            self.drag = None;
            self.last_cell = cell;
        }
        if !self.armed {
            // F10 is a local visual preference only in SAFE. When armed,
            // F10 remains a normal forwarded key for the active client.
            if focus.active().is_none()
                && local_buttons(window).is_empty()
                && window.is_key_pressed(LocalKey::F10, KeyRepeat::No)
            {
                self.portal_mode = self.portal_mode.next();
                self.effect = None;
            }
            // Rearrange only while SAFE; remote input is never forwarded
            // during a layout drag. Labels remain attached to the same peer.
            let down = window.get_mouse_down(MouseButton::Left);
            if down && !self.was_down {
                self.drag = cell.filter(|c| self.layout.label(*c).is_some());
            } else if !down
                && self.was_down
                && let (Some(from), Some(to)) = (self.drag.take(), cell)
                && self.layout.relocate(from, to)
            {
                self.painted = None;
            }
            self.was_down = down;
            return Ok(());
        }
        let prior = self.last_cell;
        self.last_cell = cell;
        if let (Some(from), Some(to), Some(at)) = (prior, cell, point)
            && from != to
        {
            // A fast pointer must not teleport through an empty cell or
            // diagonal corner and unexpectedly activate another device.
            if !self.layout.adjacent(from, to)
                || self.layout.label(from).is_none()
                || self.layout.label(to).is_none()
            {
                switch_focus(focus, None, slots, window).await?;
                *previous = Snapshot::default();
                self.armed = false;
                self.hover = None;
                self.effect = None;
                self.drag = None;
                self.was_down = window.get_mouse_down(MouseButton::Left);
                return Ok(());
            }
            self.effect = Some((at, Instant::now(), false));
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
                self.effect = None;
                self.drag = None;
                self.was_down = window.get_mouse_down(MouseButton::Left);
                return Ok(());
            }
            if target.is_none()
                && cell.is_some_and(|c| self.layout.label(c) == Some(0))
                && let Some(at) = point
            {
                self.effect = Some((at, Instant::now(), true));
            }
        }
        let Some(index) = target else {
            self.hover = None;
            if cell.is_some_and(|c| self.layout.label(c) != Some(0)) {
                self.effect = None;
            }
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
                    // Only mark a successful transition after the release and
                    // activate ACKs. Do not forward a stale pointer sample.
                    *previous = Snapshot::default();
                    self.hover = None;
                    if let Some(at) = pad::pointer(window) {
                        if pad::pointer_zone(window, self.layout).and_then(|z| z.target)
                            == Some(index)
                        {
                            self.effect = Some((at, Instant::now(), true));
                        } else {
                            switch_focus(focus, None, slots, window).await?;
                            self.effect = None;
                        }
                    }
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
        let animation = self
            .effect
            .filter(|_| self.portal_mode != portal::Mode::Off)
            .and_then(|(at, start, confirmed)| {
                let elapsed = start.elapsed().as_millis();
                (elapsed < portal::DURATION_MS).then_some((
                    at,
                    elapsed as f32 / portal::DURATION_MS as f32,
                    confirmed,
                ))
            });
        if self.effect.is_some() && animation.is_none() {
            self.effect = None;
        }
        let size = (pad::WIDTH, pad::HEIGHT);
        let restored = self.portal_patch.restore(&mut self.canvas, size);
        let changed = self.painted != Some(state);
        if changed {
            pad::render(
                self.layout,
                &mut self.canvas,
                ready,
                focus.active(),
                self.armed,
            );
            self.painted = Some(state);
        }
        if let Some((at, p, confirmed)) = animation {
            self.portal_patch
                .draw(&mut self.canvas, size, at, p, confirmed, self.portal_mode);
        }
        if changed || restored || animation.is_some() {
            window
                .update_with_buffer(&self.canvas, pad::WIDTH, pad::HEIGHT)
                .map_err(|error| anyhow::anyhow!("Spatial input pad: {error}"))?;
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
        Some(_) => "SAFE | Drag screens | F9 arm | F10 portal | ESC close",
        None => "F1-F8 select, F12 local, ESC close",
    };
    let portal = spatial.map_or(String::new(), |pad| {
        format!(" | Portal: {}", pad.portal_mode.label())
    });
    format!(
        "IntraLink INPUT | {target} | {connected}/{} connected | {controls}{portal}",
        slots.len()
    )
}

pub(super) async fn workspace(path: &Path, names: &[String], spatial: bool) -> Result<()> {
    let (_vault, secret, targets) = load_targets(path, names)?;
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
            "Spatial pad: 0 is LOCAL; drag numbered screens while SAFE to arrange the layout. Press F9 to arm; cross inside the pad with keys/buttons released. F12 disarms. F10 while SAFE cycles the local portal effect (full, reduced, off)."
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
