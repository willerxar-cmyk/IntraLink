//! Shared bounded QUIC controller workers; never capture local input.
use crate::{input_host::private_lan, local::prompt_password, routing};
use anyhow::{Context, Result, ensure};
use intralink_core::input::{Event, MAX_SLOTS};
use intralink_transport::{
    input::InputConnection,
    input_queue::{self, INTERACTIVE_BUDGET},
};
use intralink_vault::{Direction, Route, Vault};
use iroh::{Endpoint, EndpointId, SecretKey};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{sync::oneshot, task::JoinHandle, time::MissedTickBehavior};

const HEARTBEAT: Duration = Duration::from_millis(500);
const UI_LEASE: Duration = Duration::from_secs(1);
type Target = (String, EndpointId, Route);

pub(super) fn load_targets(
    path: &Path,
    names: &[String],
) -> Result<(Vault, SecretKey, Vec<Target>)> {
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
    Ok((vault, secret, targets))
}

pub(super) struct Slot {
    pub(super) name: String,
    sender: input_queue::Sender,
    ready: Arc<AtomicBool>,
    last_ui: Arc<Mutex<Instant>>,
    pub(super) task: JoinHandle<Result<()>>,
}

impl Slot {
    pub(super) fn connected(&self) -> bool {
        self.ready.load(Ordering::Acquire) && !self.task.is_finished()
    }

    pub(super) fn pulse(&self) -> Result<()> {
        *self
            .last_ui
            .lock()
            .map_err(|_| anyhow::anyhow!("Input UI lease poisoned"))? = Instant::now();
        Ok(())
    }

    pub(super) fn queue(
        &self,
        event: Event,
        acknowledgment: Option<oneshot::Sender<()>>,
    ) -> Result<()> {
        ensure!(self.connected(), "Selected input host is not connected");
        self.sender.push(event, acknowledgment).map(|_| ())
    }
}

pub(super) fn start_slot(
    endpoint: &Endpoint,
    name: String,
    identity: EndpointId,
    route: Route,
) -> Slot {
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
