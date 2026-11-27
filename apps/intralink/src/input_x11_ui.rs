//! Foreground Linux BASE using ordered X11 window events and shared QUIC workers.
use crate::{
    input_slots::{Slot, load_targets, start_slot},
    local::approve,
};
use anyhow::{Context, Result, ensure};
use intralink_core::input::{Event, Focus};
use intralink_input_x11::{Pad, PadEvent, Poll};
use intralink_transport::{bind_workspace_controller, input_queue::INTERACTIVE_BUDGET};
use std::{path::Path, time::Duration};
use tokio::{sync::oneshot, time::sleep};

const POLL: Duration = Duration::from_millis(8);

fn motion_only(poll: &Poll) -> bool {
    poll.events
        .iter()
        .all(|event| matches!(event, PadEvent::Input(Event::Move { .. })))
}

async fn confirmed(slot: &Slot, event: Event, pad: &mut Pad) -> Result<()> {
    let (ack, mut accepted) = oneshot::channel();
    slot.pulse()?;
    slot.queue(event, Some(ack))?;
    let deadline = tokio::time::Instant::now() + INTERACTIVE_BUDGET;
    loop {
        let poll = pad.poll()?;
        ensure!(
            poll.ready && !poll.interrupted && !poll.closed && motion_only(&poll),
            "Input pad lost focus or received input during switch"
        );
        slot.pulse()?;
        tokio::select! {
            biased;
            result = &mut accepted => {
                result.context("Input switch not acknowledged")?;
                // Drain once more after the ACK. A short key/button event or
                // boundary transition arriving during the wait cannot be
                // treated as input for the newly selected host.
                let final_poll = pad.poll()?;
                ensure!(final_poll.ready && !final_poll.interrupted && !final_poll.closed && motion_only(&final_poll),
                    "Local input changed during switch confirmation");
                return Ok(());
            },
            _ = tokio::time::sleep_until(deadline) => anyhow::bail!("Input switch deadline exceeded"),
            _ = sleep(POLL) => {}
        }
    }
}

async fn switch(
    focus: &mut Focus,
    target: Option<usize>,
    slots: &[Slot],
    pad: &mut Pad,
) -> Result<()> {
    if focus.active() == target {
        if let Some(index) = target {
            let (x, y) = pad
                .position()
                .context("Pointer is outside the approved window")?;
            slots[index].pulse()?;
            slots[index].queue(Event::Move { x, y }, None)?;
        }
        return Ok(());
    }
    if let Some(index) = target {
        ensure!(
            slots.get(index).is_some_and(Slot::connected),
            "Target input host unavailable"
        );
    }
    if let Some(previous) = focus.begin(target)? {
        confirmed(&slots[previous], Event::Deactivate, pad).await?;
    }
    let next = focus.released()?;
    if let Some(index) = next {
        ensure!(
            pad.neutral()?,
            "Release local keys and buttons before activating"
        );
        confirmed(&slots[index], Event::Activate, pad).await?;
        ensure!(pad.neutral()?, "Local input changed during activation");
    }
    focus.activated(next)?;
    if let Some(index) = next {
        let (x, y) = pad
            .position()
            .context("Pointer is outside the approved window")?;
        slots[index].queue(Event::Move { x, y }, None)?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Ignore,
    Local,
    Select(usize),
    Forward(Vec<Event>),
    Disconnect,
}

/// No event from an interrupted poll or a mixed selection/input batch may
/// escape. Keep ordered key and button transitions even within one poll.
fn decide(poll: Poll, active: bool) -> Decision {
    if poll.closed || (active && (!poll.ready || poll.interrupted)) {
        return Decision::Disconnect;
    }
    if !poll.ready || poll.interrupted {
        return Decision::Ignore;
    }
    if poll.events.contains(&PadEvent::Local) {
        return Decision::Local;
    }
    let selections: Vec<_> = poll
        .events
        .iter()
        .filter_map(|event| match event {
            PadEvent::Select(index) => Some(*index),
            _ => None,
        })
        .collect();
    if !selections.is_empty() {
        let mixed = selections.len() != 1 || poll.events.iter().any(|event|
            matches!(event, PadEvent::Input(input) if !matches!(input, Event::Move { .. })));
        return if mixed {
            if active {
                Decision::Disconnect
            } else {
                Decision::Ignore
            }
        } else {
            Decision::Select(selections[0])
        };
    }
    if active {
        Decision::Forward(
            poll.events
                .into_iter()
                .filter_map(|event| match event {
                    PadEvent::Input(input) => Some(input),
                    _ => None,
                })
                .collect(),
        )
    } else {
        Decision::Ignore
    }
}

pub(super) async fn workspace(path: &Path, names: &[String]) -> Result<()> {
    let (_vault, secret, targets) = load_targets(path, names)?;
    approve(
        "Open the focused local X11 input pad for these paired hosts? F1-F8 select on key release; F12 releases input; Escape closes. Leaving or obscuring the window disconnects an active host. No global hook or screen stream.",
    )?;
    let mut pad = Pad::open()?;
    let endpoint = bind_workspace_controller(secret).await?;
    let mut slots: Vec<_> = targets
        .into_iter()
        .map(|(name, identity, route)| start_slot(&endpoint, name, identity, route))
        .collect();
    for (index, slot) in slots.iter().enumerate() {
        println!("F{}: {}", index + 1, slot.name);
    }
    let mut focus = Focus::default();
    let local = async {
        loop {
            let active = focus.active();
            match decide(pad.poll()?, active.is_some()) {
                Decision::Disconnect => break,
                Decision::Ignore => {}
                Decision::Local => switch(&mut focus, None, &slots, &mut pad).await?,
                Decision::Select(index) => {
                    if slots.get(index).is_some_and(Slot::connected) && pad.neutral()? {
                        switch(&mut focus, Some(index), &slots, &mut pad).await?;
                    } else if active.is_some() {
                        // A mixed or non-neutral switch must not leave a host
                        // with potentially stale pressed state.
                        break;
                    } else {
                        println!("Target unavailable or local input held; select again.");
                    }
                }
                Decision::Forward(events) => {
                    let slot = &slots[active.context("Missing active input host")?];
                    slot.pulse()?;
                    for event in events {
                        slot.queue(event, None)?;
                    }
                }
            }
            if let Some(index) = focus.active() {
                ensure!(slots[index].connected(), "Active input host disconnected");
            }
            let name = focus
                .active()
                .map_or("LOCAL", |index| slots[index].name.as_str());
            let ready = slots.iter().filter(|slot| slot.connected()).count();
            pad.set_title(&format!(
                "IntraLink INPUT | {name} | {ready}/{} connected",
                slots.len()
            ))?;
            sleep(POLL).await;
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
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
    use intralink_core::input::Key;

    fn poll(events: Vec<PadEvent>) -> Poll {
        Poll {
            ready: true,
            events,
            ..Poll::default()
        }
    }

    #[test]
    fn window_interruption_and_mixed_selection_fail_closed() {
        let key = Key::new(4).unwrap();
        let down = Event::Key { key, down: true };
        let up = Event::Key { key, down: false };
        assert_eq!(
            decide(poll(vec![PadEvent::Input(down), PadEvent::Input(up)]), true),
            Decision::Forward(vec![down, up])
        );
        assert_eq!(
            decide(poll(vec![PadEvent::Select(1), PadEvent::Input(down)]), true),
            Decision::Disconnect
        );
        assert_eq!(
            decide(
                poll(vec![PadEvent::Select(1), PadEvent::Input(down)]),
                false
            ),
            Decision::Ignore
        );
        assert_eq!(
            decide(
                Poll {
                    ready: true,
                    interrupted: true,
                    events: vec![PadEvent::Input(down)],
                    ..Poll::default()
                },
                true
            ),
            Decision::Disconnect
        );
        assert_eq!(
            decide(poll(vec![PadEvent::Local, PadEvent::Input(down)]), true),
            Decision::Local
        );
        assert_eq!(
            decide(poll(vec![PadEvent::Select(2)]), false),
            Decision::Select(2)
        );
    }
}
