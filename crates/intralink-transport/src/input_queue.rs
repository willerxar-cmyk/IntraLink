//! Bounded single-producer/single-consumer input mailbox.
//! Only unsent, consecutive absolute pointer moves may be replaced.
//! Key/button transitions, drags, wheels and focus barriers remain FIFO.
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use intralink_core::input::{Button, Event, Key};
use tokio::sync::{Notify, oneshot};

pub const CAPACITY: usize = 16;
pub const INTERACTIVE_BUDGET: Duration = Duration::from_millis(500);

pub struct Command {
    pub event: Event,
    pub acknowledgment: Option<oneshot::Sender<()>>,
    queued_at: Instant,
}

impl Command {
    /// One local budget includes both queue residence and wire acceptance.
    /// No sender timestamp is serialized or trusted across machines.
    pub fn remaining(&self) -> Result<Duration> {
        let left = INTERACTIVE_BUDGET.saturating_sub(self.queued_at.elapsed());
        ensure!(!left.is_zero(), "Input command expired in local queue");
        Ok(left)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enqueue {
    Added,
    ReplacedMotion,
}

struct Queue {
    commands: VecDeque<Command>,
    keys: BTreeSet<Key>,
    buttons: BTreeSet<Button>,
    producer_closed: bool,
    consumer_closed: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    notify: Notify,
}

pub struct Sender(Arc<Shared>);
pub struct Receiver(Arc<Shared>);

pub fn channel() -> (Sender, Receiver) {
    let shared = Arc::new(Shared {
        queue: Mutex::new(Queue {
            commands: VecDeque::with_capacity(CAPACITY),
            keys: BTreeSet::new(),
            buttons: BTreeSet::new(),
            producer_closed: false,
            consumer_closed: false,
        }),
        notify: Notify::new(),
    });
    (Sender(shared.clone()), Receiver(shared))
}

impl Sender {
    pub fn push(
        &self,
        event: Event,
        acknowledgment: Option<oneshot::Sender<()>>,
    ) -> Result<Enqueue> {
        let command = Command {
            event,
            acknowledgment,
            queued_at: Instant::now(),
        };
        let mut queue = self
            .0
            .queue
            .lock()
            .map_err(|_| anyhow::anyhow!("Input queue poisoned"))?;
        ensure!(
            !queue.consumer_closed && !queue.producer_closed,
            "Input queue closed"
        );
        // A held key or mouse button may give motion path semantics, e.g.
        // selection, drawing or dragging. Do not coalesce in that case.
        if queue.keys.is_empty()
            && queue.buttons.is_empty()
            && matches!(event, Event::Move { .. })
            && command.acknowledgment.is_none()
            && let Some(tail) = queue.commands.back_mut()
            && matches!(tail.event, Event::Move { .. })
            && tail.acknowledgment.is_none()
        {
            *tail = command;
            return Ok(Enqueue::ReplacedMotion);
        }
        ensure!(
            queue.commands.len() < CAPACITY,
            "Input queue full; refusing stale input"
        );
        match event {
            Event::Activate | Event::Deactivate | Event::End => {
                queue.keys.clear();
                queue.buttons.clear();
            }
            Event::Key { key, down: true } => {
                queue.keys.insert(key);
            }
            Event::Key { key, down: false } => {
                queue.keys.remove(&key);
            }
            Event::Button { button, down: true } => {
                queue.buttons.insert(button);
            }
            Event::Button {
                button,
                down: false,
            } => {
                queue.buttons.remove(&button);
            }
            _ => {}
        }
        queue.commands.push_back(command);
        drop(queue);
        self.0.notify.notify_one();
        Ok(Enqueue::Added)
    }
}

impl Receiver {
    /// Cancellation safe: a pending receive never removes a command.
    /// There is only one consumer; notify_one retains a wake permit.
    pub async fn recv(&mut self) -> Result<Option<Command>> {
        loop {
            {
                let mut queue = self
                    .0
                    .queue
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Input queue poisoned"))?;
                // On owner shutdown do not drain stale queued input.
                if queue.producer_closed || queue.consumer_closed {
                    return Ok(None);
                }
                if let Some(command) = queue.commands.pop_front() {
                    return Ok(Some(command));
                }
            }
            self.0.notify.notified().await;
        }
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.0.queue.lock() {
            queue.producer_closed = true;
            queue.commands.clear();
        }
        self.0.notify.notify_one();
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.0.queue.lock() {
            queue.consumer_closed = true;
            queue.commands.clear();
        }
        self.0.notify.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    fn movement(x: u16) -> Event {
        Event::Move { x, y: 0 }
    }
    fn button(down: bool) -> Event {
        Event::Button {
            button: Button::Left,
            down,
        }
    }

    #[tokio::test]
    async fn movement_replaces_only_unsent_tail_and_never_drag_or_click_barriers() -> Result<()> {
        let (tx, mut rx) = channel();
        assert_eq!(tx.push(Event::Activate, None)?, Enqueue::Added);
        for x in 0..100 {
            let expected = if x == 0 {
                Enqueue::Added
            } else {
                Enqueue::ReplacedMotion
            };
            assert_eq!(tx.push(movement(x), None)?, expected);
        }
        tx.push(button(true), None)?;
        assert_eq!(tx.push(movement(101), None)?, Enqueue::Added);
        assert_eq!(tx.push(movement(102), None)?, Enqueue::Added);
        tx.push(button(false), None)?;
        tx.push(movement(103), None)?;
        tx.push(movement(104), None)?;
        tx.push(Event::Deactivate, None)?;
        for expected in [
            Event::Activate,
            movement(99),
            button(true),
            movement(101),
            movement(102),
            button(false),
            movement(104),
            Event::Deactivate,
        ] {
            assert_eq!(rx.recv().await?.context("Missing command")?.event, expected);
        }
        Ok(())
    }

    #[tokio::test]
    async fn key_and_ack_barriers_are_preserved() -> Result<()> {
        let (tx, mut rx) = channel();
        let key = Key::new(225).unwrap();
        let (ack, _accepted) = oneshot::channel();
        tx.push(movement(1), Some(ack))?;
        tx.push(movement(2), None)?;
        tx.push(Event::Key { key, down: true }, None)?;
        tx.push(movement(3), None)?;
        tx.push(movement(4), None)?;
        tx.push(Event::Key { key, down: false }, None)?;
        tx.push(movement(5), None)?;
        tx.push(movement(6), None)?;
        for expected in [
            movement(1),
            movement(2),
            Event::Key { key, down: true },
            movement(3),
            movement(4),
            Event::Key { key, down: false },
            movement(6),
        ] {
            assert_eq!(rx.recv().await?.context("Missing command")?.event, expected);
        }
        Ok(())
    }

    #[tokio::test]
    async fn saturation_and_shutdown_fail_closed() -> Result<()> {
        let (tx, mut rx) = channel();
        for _ in 0..CAPACITY {
            tx.push(Event::Heartbeat, None)?;
        }
        assert!(tx.push(Event::Deactivate, None).is_err());
        drop(tx);
        assert!(rx.recv().await?.is_none());
        let (tx, rx) = channel();
        let (ack, accepted) = oneshot::channel();
        tx.push(Event::Activate, Some(ack))?;
        drop(rx);
        assert!(accepted.await.is_err());
        assert!(tx.push(movement(1), None).is_err());
        Ok(())
    }

    #[test]
    fn local_command_age_reduces_remaining_wire_budget() {
        let fresh = Command {
            event: movement(0),
            acknowledgment: None,
            queued_at: Instant::now(),
        };
        assert!(
            fresh
                .remaining()
                .is_ok_and(|value| value <= INTERACTIVE_BUDGET)
        );
        let old = Command {
            event: movement(0),
            acknowledgment: None,
            queued_at: Instant::now() - INTERACTIVE_BUDGET - Duration::from_millis(1),
        };
        assert!(old.remaining().is_err());
    }

    #[tokio::test]
    async fn canceled_wait_does_not_consume_next_command() -> Result<()> {
        let (tx, mut rx) = channel();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), rx.recv())
                .await
                .is_err()
        );
        tx.push(movement(8), None)?;
        assert_eq!(
            rx.recv().await?.context("Missing command")?.event,
            movement(8)
        );
        Ok(())
    }
}
