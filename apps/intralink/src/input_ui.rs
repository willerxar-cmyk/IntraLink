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

#[path = "input_desktop_edges.rs"]
mod desktop_edges;
pub(super) use desktop_edges::SourceMode;
#[path = "input_display_review.rs"]
mod display_review;
#[path = "input_edge_check.rs"]
mod edge_check;
pub(super) use edge_check::run as edge_check;
#[path = "input_keys.rs"]
mod keys;
#[path = "input_layout_store.rs"]
mod layout_store;
#[path = "input_pad.rs"]
mod pad;
#[path = "input_portal.rs"]
mod portal;
#[path = "input_topology_canvas.rs"]
mod topology_canvas;
#[path = "input_topology_editor.rs"]
mod topology_editor;
#[path = "input_topology_pad.rs"]
mod topology_pad;
#[path = "input_topology_store.rs"]
pub(super) mod topology_store;
#[path = "input_workspace.rs"]
mod workspace_run;
pub(super) use workspace_run::{WorkspaceMode, configure_topology, workspace};

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
        self.confirmed_guarded(event, window, |_| true).await
    }

    async fn confirmed_guarded(
        &self,
        event: Event,
        window: &mut Window,
        guard: impl Fn(&Window) -> bool,
    ) -> Result<()> {
        ensure!(
            window.is_open()
                && window.is_active()
                && !window.is_key_down(LocalKey::Escape)
                && guard(window),
            "Local input changed before acknowledged action"
        );
        let (acknowledgment, mut accepted) = oneshot::channel();
        self.pulse()?;
        self.queue(event, Some(acknowledgment))?;
        let deadline = tokio::time::Instant::now() + INTERACTIVE_BUDGET;
        loop {
            window.update();
            ensure!(
                window.is_open()
                    && window.is_active()
                    && !window.is_key_down(LocalKey::Escape)
                    && guard(window),
                "Local input pad stopped, lost focus or changed during switch"
            );
            self.pulse()?;
            tokio::select! {
                result = &mut accepted => {
                    result.context("Input switch not acknowledged")?;
                    window.update();
                    ensure!(window.is_open() && window.is_active() && !window.is_key_down(LocalKey::Escape) && guard(window),"Local input changed at acknowledgment");
                    return Ok(());
                },
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
        self.forward_with_keys(window, slot, position, local_keys(window))
    }

    fn forward_with_keys(
        &mut self,
        window: &Window,
        slot: &Slot,
        position: (u16, u16),
        current: BTreeSet<Key>,
    ) -> Result<()> {
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
    painted: Option<(u8, Option<usize>, bool, portal::Mode)>,
    drag: Option<usize>,
    was_down: bool,
    pressed_control: Option<pad::LocalControl>,
    last_cell: Option<usize>,
    effect: Option<((usize, usize), Instant, bool)>,
    portal_mode: portal::Mode,
    portal_patch: portal::Patch,
    layout_store: layout_store::Store,
    layout_status: &'static str,
    layout_blocked: bool,
}

impl Spatial {
    fn new(store: layout_store::Store) -> Self {
        let (preference, layout_status, layout_blocked) = match store.load() {
            Ok(Some(preference)) => (preference, "LAYOUT RESTORED", false),
            Ok(None) => (store.defaults(), "NEW LAYOUT", false),
            Err(error) => {
                eprintln!("Saved KVM layout could not be read: {error:#}. Reset to replace it.");
                (store.defaults(), "LAYOUT UNREADABLE - RESET", true)
            }
        };
        Self {
            layout: preference.layout,
            armed: false,
            hover: None,
            canvas: vec![0; pad::WIDTH * pad::HEIGHT],
            painted: None,
            drag: None,
            was_down: true,
            pressed_control: None,
            last_cell: None,
            effect: None,
            portal_mode: preference.portal,
            portal_patch: portal::Patch::default(),
            layout_store: store,
            layout_status,
            layout_blocked,
        }
    }

    fn persist_layout(&mut self) {
        if self.armed || self.layout_blocked {
            return;
        }
        let preference = layout_store::Preference {
            layout: self.layout,
            portal: self.portal_mode,
        };
        self.layout_status = match self.layout_store.save(preference) {
            Ok(()) => "LAYOUT SAVED",
            Err(error) => {
                eprintln!("KVM layout not saved: {error:#}");
                "LAYOUT NOT SAVED"
            }
        };
        self.painted = None;
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
            self.was_down = true;
            self.pressed_control = None;
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
            self.pressed_control = None;
            self.last_cell = None;
            self.effect = None;
            return Ok(());
        }
        if toggle
            && self.painted.is_some()
            && cell.is_some_and(|c| self.layout.label(c) == Some(0))
            && local_keys(window).is_empty()
            && local_buttons(window).is_empty()
        {
            self.armed = true;
            self.hover = None;
            self.drag = None;
            self.pressed_control = None;
            self.last_cell = cell;
        }
        if !self.armed {
            let before = (self.layout, self.portal_mode);
            let mut reset = false;
            // F10 is a local visual preference only in SAFE. When armed,
            // F10 remains a normal forwarded key for the active client.
            if focus.active().is_none()
                && local_buttons(window).is_empty()
                && window.is_key_pressed(LocalKey::F10, KeyRepeat::No)
            {
                self.portal_mode = self.portal_mode.next();
                self.effect = None;
            }
            // Controls require a complete click inside the local BASE card.
            // Other drags may only rearrange the disarmed layout.
            let down = window.get_mouse_down(MouseButton::Left);
            let control = point.and_then(|p| pad::local_control(self.layout, p));
            if down && !self.was_down {
                self.pressed_control = control.filter(|_| self.painted.is_some());
                self.drag = if control.is_none() {
                    cell.filter(|c| self.layout.label(*c).is_some())
                } else {
                    None
                };
            } else if !down && self.was_down {
                let clicked = self.pressed_control.take().filter(|c| Some(*c) == control);
                match clicked {
                    Some(pad::LocalControl::Arm)
                        if focus.active().is_none()
                            && local_keys(window).is_empty()
                            && local_buttons(window).is_empty() =>
                    {
                        self.armed = true;
                        self.last_cell = cell;
                        self.hover = None;
                    }
                    Some(pad::LocalControl::Portal) => {
                        self.portal_mode = self.portal_mode.next();
                        self.effect = None;
                    }
                    Some(pad::LocalControl::Reset) => {
                        let preference = self.layout_store.defaults();
                        self.layout = preference.layout;
                        self.portal_mode = preference.portal;
                        self.layout_blocked = false;
                        self.effect = None;
                        reset = true;
                    }
                    _ => {
                        if let (Some(from), Some(to)) = (self.drag.take(), cell)
                            && self.layout.relocate(from, to)
                        {
                            self.painted = None;
                        }
                    }
                }
                self.drag = None;
            }
            self.was_down = down;
            if reset || before != (self.layout, self.portal_mode) {
                self.persist_layout();
            }
            return Ok(());
        }
        // The armed local stop button never receives clicks from a remote
        // zone or from an active client. The F12 emergency stop remains.
        let down = window.get_mouse_down(MouseButton::Left);
        let local_stop = focus.active().is_none()
            && point.and_then(|p| pad::local_control(self.layout, p))
                == Some(pad::LocalControl::Arm);
        if down && !self.was_down {
            self.pressed_control = local_stop.then_some(pad::LocalControl::Arm);
        } else if !down && self.was_down {
            let stop = self.pressed_control.take() == Some(pad::LocalControl::Arm) && local_stop;
            if stop {
                self.armed = false;
                self.hover = None;
                self.effect = None;
                self.last_cell = None;
                self.was_down = false;
                return Ok(());
            }
        }
        if !local_stop {
            self.pressed_control = None;
        }
        self.was_down = down;
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
        let state = (ready, focus.active(), self.armed, self.portal_mode);
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
            let names: Vec<_> = slots.iter().map(|slot| slot.name.as_str()).collect();
            pad::render(
                self.layout,
                &mut self.canvas,
                pad::View {
                    names: &names,
                    ready,
                    active: focus.active(),
                    armed: self.armed,
                    portal: self.portal_mode.label(),
                    layout_status: self.layout_status,
                },
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

fn title(
    slots: &[Slot],
    focus: &Focus,
    spatial: Option<&Spatial>,
    geometry: Option<&topology_pad::Pad>,
) -> String {
    let target = focus
        .active()
        .map_or("LOCAL", |index| slots[index].name.as_str());
    let connected = slots.iter().filter(|slot| slot.connected()).count();
    let controls = match spatial {
        Some(pad) if pad.armed => "SPATIAL ARMED | 0 local, F12 disarm, ESC close",
        Some(_) => "SAFE | Drag screens | Click START or F9 | F10 portal | ESC close",
        None if geometry.is_some_and(|pad| pad.armed()) => "GEOMETRY ARMED | F12 stop | ESC close",
        None if geometry.is_some() => "GEOMETRY SAFE | Click BASE or F9 | ESC close",
        None => "F1-F8 select, F12 local, ESC close",
    };
    let portal = spatial.map_or(String::new(), |pad| {
        format!(
            " | Portal: {} | {}",
            pad.portal_mode.label(),
            pad.layout_status
        )
    });
    format!(
        "IntraLink INPUT | {target} | {connected}/{} connected | {controls}{portal}",
        slots.len()
    )
}
