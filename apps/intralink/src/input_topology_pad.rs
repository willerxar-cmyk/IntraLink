//! Geometry-aware, strictly window-scoped KVM. No global pointer hook.
//! The pure gate cannot grant a session; the existing ordered ACK barrier
//! remains authoritative for the actual remote focus.
use super::{
    Slot, Snapshot, local_buttons, portal, switch_focus,
    topology_canvas::{self as ui, Cards, LIVE_AREA, Projection, Sample},
    topology_store::{self, Config, Observed},
};
use anyhow::{Context, Result, ensure};
use intralink_core::{
    display_geometry,
    input::{Event, Focus},
    overview::{HEIGHT, WIDTH},
    screen_topology::{Topology, Transfer},
};
use minifb::{Key, KeyRepeat, MouseButton, Window};
use std::time::{Duration, Instant};

const DWELL: Duration = Duration::from_millis(80);

#[derive(Clone, Copy)]
struct Pending {
    transfer: Transfer,
    since: Instant,
}

enum Decision {
    Stop,
    Local,
    Forward(Sample),
    Cross(Transfer),
    Wait,
    Activate(Transfer),
}

#[derive(Default)]
struct Gate {
    last: Option<Sample>,
    pending: Option<Pending>,
}

fn target(id: u8) -> Option<usize> {
    usize::from(id).checked_sub(1)
}
fn available(id: u8, ready: u8) -> bool {
    target(id).is_none_or(|i| i < 8 && ready & (1 << i) != 0)
}

impl Gate {
    fn armed(&self) -> bool {
        self.last.is_some()
    }
    fn stop(&mut self) {
        self.last = None;
        self.pending = None;
    }
    fn arm(&mut self, sample: Sample) {
        self.stop();
        if sample.id == 0 {
            self.last = Some(sample);
        }
    }

    fn released(&mut self, transfer: Transfer, sample: Sample, now: Instant) {
        if sample.id != transfer.target {
            self.stop();
            return;
        }
        self.last = Some(sample);
        self.pending = target(transfer.target).map(|_| Pending {
            transfer,
            since: now,
        });
    }

    fn activated(&mut self, sample: Sample) {
        self.pending = None;
        self.last = Some(sample);
    }

    fn decide(
        &mut self,
        t: &Topology,
        sample: Option<Sample>,
        active: Option<usize>,
        ready: u8,
        held: bool,
        now: Instant,
    ) -> Decision {
        let (Some(previous), Some(current)) = (self.last, sample) else {
            self.stop();
            return Decision::Stop;
        };
        let next = if !available(current.id, ready) {
            Decision::Stop
        } else if current.id == previous.id {
            if let Some(pending) = self.pending {
                if held || active.is_some() || pending.transfer.target != current.id {
                    Decision::Stop
                } else if now.saturating_duration_since(pending.since) >= DWELL {
                    Decision::Activate(pending.transfer)
                } else {
                    Decision::Wait
                }
            } else if active != target(current.id) {
                Decision::Stop
            } else if current.id == 0 {
                Decision::Local
            } else {
                Decision::Forward(current)
            }
        } else if held || self.pending.is_some() || active != target(previous.id) {
            Decision::Stop
        } else {
            t.trace(previous.id, previous.point, current.point)
                .map_or(Decision::Stop, Decision::Cross)
        };
        if matches!(next, Decision::Stop) {
            self.stop();
        } else {
            self.last = Some(current);
        }
        next
    }
}

fn neutral(window: &Window, arming_key: bool) -> bool {
    window
        .get_keys()
        .into_iter()
        .all(|key| arming_key && key == Key::F9)
        && local_buttons(window).is_empty()
}

pub(super) struct Pad {
    config: Config,
    observed: Observed,
    authenticated: bool,
    checked_base: Instant,
    topology: Topology,
    projection: Projection,
    gate: Gate,
    pressed_base: bool,
    was_down: bool,
    canvas: Vec<u32>,
    painted: Option<(u8, Option<usize>, bool, bool)>,
    effect: Option<((usize, usize), Instant, bool)>,
    patch: portal::Patch,
}

impl Pad {
    pub fn new(config: Config, observed: Observed, authenticated: bool) -> Result<Self> {
        let topology =
            display_geometry::topology(&config.placements).context("Invalid saved topology")?;
        ensure!(topology.connected(), "Topology has disconnected screens");
        let projection =
            Projection::new(&topology, LIVE_AREA).context("Topology cannot be displayed")?;
        Ok(Self {
            config,
            observed,
            authenticated,
            checked_base: Instant::now(),
            topology,
            projection,
            gate: Gate::default(),
            pressed_base: false,
            was_down: true,
            canvas: vec![0; WIDTH * HEIGHT],
            painted: None,
            effect: None,
            patch: portal::Patch::default(),
        })
    }

    pub fn armed(&self) -> bool {
        self.gate.armed()
    }
    fn sample(&self, window: &Window) -> Option<Sample> {
        super::pad::pointer(window).and_then(|p| self.projection.sample(&self.topology, p))
    }

    async fn stop(
        &mut self,
        focus: &mut Focus,
        previous: &mut Snapshot,
        slots: &[Slot],
        window: &mut Window,
    ) -> Result<()> {
        self.gate.stop();
        self.effect = None;
        self.pressed_base = false;
        self.was_down = true;
        switch_focus(focus, None, slots, window).await?;
        *previous = Snapshot::default();
        Ok(())
    }

    pub async fn step(
        &mut self,
        focus: &mut Focus,
        previous: &mut Snapshot,
        slots: &[Slot],
        window: &mut Window,
    ) -> Result<()> {
        if self.authenticated {
            ensure!(
                self.config.placements.len() == slots.len() + 1
                    && slots
                        .iter()
                        .zip(self.config.placements.iter().skip(1))
                        .all(|(slot, p)| slot.display() == Some(p.display)),
                "Authenticated CLIENT geometry session ended; reopen workspace"
            );
        }
        if self.checked_base.elapsed() >= Duration::from_millis(500) {
            ensure!(
                topology_store::observe_base()? == self.observed,
                "BASE geometry changed; reopen workspace"
            );
            self.checked_base = Instant::now();
        }
        let ready = slots.iter().enumerate().fold(0u8, |mask, (i, slot)| {
            mask | if slot.connected() { 1 << i } else { 0 }
        });
        if !window.is_active() || window.is_key_down(Key::F12) {
            return self.stop(focus, previous, slots, window).await;
        }
        let sample = self.sample(window);
        let on_base = sample.is_some_and(|s| s.id == 0);
        let down = window.get_mouse_down(MouseButton::Left);
        let f9 = window.is_key_pressed(Key::F9, KeyRepeat::No);
        if !self.armed() {
            if down && !self.was_down {
                self.pressed_base = on_base && window.get_keys().is_empty();
            }
            let click = !down && self.was_down && std::mem::take(&mut self.pressed_base) && on_base;
            self.was_down = down;
            if self.painted.is_some()
                && on_base
                && focus.active().is_none()
                && ((click && neutral(window, false)) || (f9 && neutral(window, true)))
            {
                ensure!(
                    topology_store::observe_base()? == self.observed,
                    "BASE monitor changed; reopen geometry editor"
                );
                self.gate.arm(sample.expect("BASE sample"));
            }
            return Ok(());
        }
        if f9 {
            return self.stop(focus, previous, slots, window).await;
        }
        let held =
            !previous.keys.is_empty() || !previous.buttons.is_empty() || !neutral(window, false);
        match self.gate.decide(
            &self.topology,
            sample,
            focus.active(),
            ready,
            held,
            Instant::now(),
        ) {
            Decision::Stop => self.stop(focus, previous, slots, window).await?,
            Decision::Local | Decision::Wait => {}
            Decision::Forward(s) => {
                let index = target(s.id).context("Invalid forwarded geometry slot")?;
                previous.forward(window, &slots[index], s.normalized)?;
            }
            Decision::Cross(transfer) => {
                let at = super::pad::pointer(window).context("Missing crossing point")?;
                self.effect = Some((at, Instant::now(), false));
                // Always acknowledge release before the new dwell interval.
                switch_focus(focus, None, slots, window).await?;
                *previous = Snapshot::default();
                let fresh = self.sample(window);
                if !window.is_active()
                    || !neutral(window, false)
                    || fresh.is_none_or(|s| s.id != transfer.target)
                {
                    self.stop(focus, previous, slots, window).await?;
                    return Ok(());
                }
                self.gate
                    .released(transfer, fresh.expect("fresh destination"), Instant::now());
                if transfer.target == 0 {
                    self.effect = Some((at, Instant::now(), true));
                }
            }
            Decision::Activate(transfer) => {
                let index = target(transfer.target).context("Invalid activation slot")?;
                ensure!(
                    focus.active().is_none() && slots[index].connected(),
                    "Geometry activation requires acknowledged local focus"
                );
                let projection = self.projection;
                let topology = &self.topology;
                let guard = |window: &Window| {
                    !window.is_key_down(Key::F12)
                        && neutral(window, false)
                        && super::pad::pointer(window)
                            .and_then(|p| projection.sample(topology, p))
                            .is_some_and(|s| s.id == transfer.target)
                };
                ensure!(
                    focus.begin(Some(index))?.is_none(),
                    "Unexpected previous input owner"
                );
                ensure!(
                    focus.released()? == Some(index),
                    "Unexpected geometry activation target"
                );
                slots[index]
                    .confirmed_guarded(Event::Activate, window, guard)
                    .await?;
                focus.activated(Some(index))?;
                slots[index]
                    .confirmed_guarded(
                        Event::Move {
                            x: transfer.x,
                            y: transfer.y,
                        },
                        window,
                        guard,
                    )
                    .await?;
                let fresh = self
                    .sample(window)
                    .context("Destination left after activation")?;
                ensure!(
                    fresh.id == transfer.target && neutral(window, false),
                    "Destination changed after activation"
                );
                *previous = Snapshot::default();
                previous.position = Some((transfer.x, transfer.y));
                self.gate.activated(fresh);
                let at = super::pad::pointer(window).context("Missing destination pointer")?;
                self.effect = Some((at, Instant::now(), true));
            }
        }
        Ok(())
    }

    pub fn paint(&mut self, slots: &[Slot], focus: &Focus, window: &mut Window) -> Result<()> {
        let ready = slots.iter().enumerate().fold(0u8, |mask, (i, slot)| {
            mask | if slot.connected() { 1 << i } else { 0 }
        });
        let state = (
            ready,
            focus.active(),
            self.armed(),
            self.gate.pending.is_some(),
        );
        let changed = self.painted != Some(state);
        let size = (WIDTH, HEIGHT);
        let restored = self.patch.restore(&mut self.canvas, size);
        let animation = self
            .effect
            .filter(|_| self.config.portal != portal::Mode::Off)
            .and_then(|(at, start, confirmed)| {
                let elapsed = start.elapsed().as_millis();
                (elapsed < portal::DURATION_MS).then_some((
                    at,
                    elapsed as f32 / portal::DURATION_MS as f32,
                    confirmed,
                ))
            });
        if animation.is_none() {
            self.effect = None;
        }
        if changed {
            self.canvas.fill(ui::BG);
            ui::label(
                &mut self.canvas,
                16,
                14,
                if state.2 {
                    "INTRALINK / GEOMETRY ARMED"
                } else {
                    "INTRALINK / GEOMETRY SAFE"
                },
                2,
                ui::ACCENT,
            );
            let mut names = vec!["BASE"];
            names.extend(slots.iter().map(|slot| slot.name.as_str()));
            ui::render_cards(
                &mut self.canvas,
                &self.topology,
                self.projection,
                Cards {
                    names: &names,
                    placements: &self.config.placements,
                    confirmed: (1 << self.config.placements.len()) - 1,
                    selected: None,
                    ready,
                    active: focus.active(),
                    editing: false,
                },
            );
            ui::label(
                &mut self.canvas,
                16,
                494,
                if state.2 {
                    if state.3 {
                        "WAITING FOR SHARED EDGE ACTIVATION"
                    } else {
                        "F12 STOP / ESC CLOSE / WINDOW ONLY"
                    }
                } else {
                    "CLICK BASE OR PRESS F9 INSIDE BASE TO START"
                },
                2,
                ui::MUTED,
            );
            self.painted = Some(state);
        }
        if let Some((at, p, confirmed)) = animation {
            self.patch
                .draw(&mut self.canvas, size, at, p, confirmed, self.config.portal);
        }
        if changed || restored || animation.is_some() {
            window
                .update_with_buffer(&self.canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("Geometry input pad: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use intralink_core::screen_topology::Screen;
    fn sample(t: &Topology, x: i32, y: i32) -> Sample {
        let id = t.at(x, y).unwrap();
        Sample {
            id,
            point: (x, y),
            normalized: t.normalized(id, x, y).unwrap(),
        }
    }
    fn topology() -> Topology {
        Topology::new(&[
            Screen {
                id: 0,
                x: 0,
                y: 0,
                width: 1000,
                height: 1000,
            },
            Screen {
                id: 1,
                x: 1000,
                y: 200,
                width: 500,
                height: 500,
            },
            Screen {
                id: 2,
                x: 1500,
                y: 200,
                width: 500,
                height: 500,
            },
        ])
        .unwrap()
    }
    #[test]
    fn dwell_starts_only_after_release_and_never_carries_a_held_chord() {
        let t = topology();
        let now = Instant::now();
        let base = sample(&t, 900, 300);
        let first = sample(&t, 1100, 400);
        let second = sample(&t, 1600, 400);
        let mut gate = Gate::default();
        gate.arm(base);
        let Decision::Cross(transfer) = gate.decide(&t, Some(first), None, 3, false, now) else {
            panic!("Expected shared edge");
        };
        assert_eq!(transfer.target, 1);
        gate.released(transfer, first, now + Duration::from_millis(200));
        assert!(matches!(
            gate.decide(
                &t,
                Some(first),
                None,
                3,
                false,
                now + Duration::from_millis(279)
            ),
            Decision::Wait
        ));
        assert!(matches!(
            gate.decide(
                &t,
                Some(first),
                None,
                3,
                false,
                now + Duration::from_millis(280)
            ),
            Decision::Activate(_)
        ));
        gate.activated(first);
        assert!(matches!(
            gate.decide(&t, Some(first), Some(0), 3, false, now),
            Decision::Forward(_)
        ));
        assert!(matches!(
            gate.decide(&t, Some(second), Some(0), 3, true, now),
            Decision::Stop
        ));
        assert!(!gate.armed());
    }

    #[test]
    fn no_gap_skip_unavailable_or_pre_activation_second_cross() {
        let t = topology();
        let now = Instant::now();
        let mut gate = Gate::default();
        for (from, to, ready) in [
            (sample(&t, 900, 300), Some(sample(&t, 1600, 300)), 3),
            (sample(&t, 900, 0), Some(sample(&t, 1100, 210)), 3),
            (sample(&t, 900, 300), Some(sample(&t, 1100, 300)), 0),
            (sample(&t, 900, 300), None, 3),
        ] {
            gate.arm(from);
            assert!(matches!(
                gate.decide(&t, to, None, ready, false, now),
                Decision::Stop
            ));
        }
        let a = sample(&t, 900, 300);
        let b = sample(&t, 1100, 300);
        gate.arm(a);
        let Decision::Cross(c) = gate.decide(&t, Some(b), None, 3, false, now) else {
            panic!()
        };
        gate.released(c, b, now);
        assert!(matches!(
            gate.decide(&t, Some(sample(&t, 1600, 300)), None, 3, false, now),
            Decision::Stop
        ));
    }
}
