//! Separate, explicitly enabled software-KVM ALPN. Input is never granted by
//! a video session or a diagnostic probe. The local consumer owns permission.
use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use intralink_core::input::{
    ALPN, Event, HELLO, MAX_EVENTS_PER_SECOND, MAX_SECONDS, READY, RECORD_BYTES, Record, State,
};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, SecretKey,
    endpoint::{Connection, RecvStream, SendStream},
};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, timeout},
};

use crate::{HANDSHAKE_LIMIT, bind_for_protocol, direct_candidates};

const ACCEPT_LIMIT: Duration = Duration::from_secs(120);
const IO_LIMIT: Duration = Duration::from_secs(3);

/// A verified, ordered event awaiting the explicitly authorized local sink.
/// An acknowledgment means local acceptance, never an on-screen guarantee.
pub struct Delivery {
    pub record: Record,
    pub accepted: oneshot::Sender<Result<()>>,
}

pub async fn bind_input(secret: SecretKey, address: SocketAddr) -> Result<Endpoint> {
    bind_for_protocol(secret, address, Some(ALPN)).await
}

fn remaining(deadline: Instant) -> Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    ensure!(!left.is_zero(), "Input session expired");
    Ok(left.min(IO_LIMIT))
}

/// One authenticated controller, one local input approval, one bounded session.
/// The caller must display consent and a visible local stop affordance before
/// calling this function. The receiver must release its held state on all exits.
pub async fn serve_input(
    endpoint: &Endpoint,
    approved: EndpointId,
    seconds: u16,
    sink: mpsc::Sender<Delivery>,
) -> Result<u32> {
    ensure!(
        (1..=MAX_SECONDS).contains(&seconds),
        "Invalid input session duration"
    );
    let incoming = timeout(ACCEPT_LIMIT, endpoint.accept())
        .await?
        .context("Input listener closed")?;
    let connection = timeout(HANDSHAKE_LIMIT, async { incoming.await }).await??;
    if connection.remote_id() != approved {
        connection.close(1u32.into(), b"input peer not approved");
        bail!("Input peer not approved");
    }
    let result = receive(&connection, seconds, sink).await;
    if result.is_err() {
        connection.close(2u32.into(), b"input session stopped");
    }
    result
}

async fn receive(
    connection: &Connection,
    seconds: u16,
    sink: mpsc::Sender<Delivery>,
) -> Result<u32> {
    let (mut tx, mut rx) = timeout(IO_LIMIT, connection.accept_bi()).await??;
    timeout(IO_LIMIT, async {
        let mut hello = [0; HELLO.len()];
        rx.read_exact(&mut hello).await?;
        ensure!(&hello == HELLO, "Invalid input handshake");
        tx.write_all(READY).await?;
        tx.write_all(&seconds.to_be_bytes()).await?;
        Ok::<(), anyhow::Error>(())
    })
    .await??;
    let deadline = Instant::now() + Duration::from_secs(u64::from(seconds));
    let mut state = State::default();
    let mut count = 0;
    let mut rate_start = Instant::now();
    let mut rate_count = 0u32;
    loop {
        let mut bytes = [0; RECORD_BYTES];
        timeout(remaining(deadline)?, rx.read_exact(&mut bytes))
            .await
            .context("Input idle/session deadline exceeded")??;
        let record = Record::decode(bytes)?;
        state.validate(record)?;
        if rate_start.elapsed() >= Duration::from_secs(1) {
            rate_start = Instant::now();
            rate_count = 0;
        }
        rate_count += 1;
        ensure!(
            rate_count <= MAX_EVENTS_PER_SECOND,
            "Input event rate exceeded"
        );
        // One outstanding event only; no remote-controlled queue, resource
        // expansion, or acknowledgment before the local sink accepts it.
        let (accepted, result) = oneshot::channel();
        // A lost controller cancels the acceptance handle immediately rather
        // than leaving the native sink with an apparently live stale event.
        // Bound the complete delivery/acceptance/ACK phase, not each substep.
        timeout(remaining(deadline)?, async {
            tokio::select! {
                biased;
                _ = connection.closed() => bail!("Controller disconnected during input acceptance"),
                result = async {
                    sink.send(Delivery { record, accepted }).await
                        .map_err(|_| anyhow::anyhow!("Local input sink closed"))?;
                    result.await.context("Local input acceptance dropped")?
                        .context("Local input rejected")?;
                    state.commit(record)?;
                    tx.write_all(&record.sequence.to_be_bytes()).await?;
                    Ok::<(), anyhow::Error>(())
                } => result,
            }
        })
        .await
        .context("Input delivery/acceptance deadline exceeded")??;
        count += 1;
        if record.event == Event::End {
            tx.finish()?;
            let _ = timeout(IO_LIMIT, connection.closed()).await;
            return Ok(count);
        }
    }
}

pub struct InputConnection {
    connection: Connection,
    tx: SendStream,
    rx: RecvStream,
    state: State,
    deadline: Instant,
    closed: bool,
}

impl InputConnection {
    pub async fn connect(
        endpoint: &Endpoint,
        expected: EndpointId,
        candidates: &[SocketAddr],
    ) -> Result<Self> {
        let candidates = direct_candidates(candidates)?;
        let target = candidates
            .into_iter()
            .fold(EndpointAddr::new(expected), |target, addr| {
                target.with_ip_addr(addr)
            });
        let connection = timeout(HANDSHAKE_LIMIT, endpoint.connect(target, ALPN)).await??;
        ensure!(
            connection.remote_id() == expected,
            "Input host identity mismatch"
        );
        let handshake = timeout(IO_LIMIT, async {
            let (mut tx, mut rx) = connection.open_bi().await?;
            tx.write_all(HELLO).await?;
            let mut response = [0; READY.len()];
            rx.read_exact(&mut response).await?;
            ensure!(&response == READY, "Invalid input acceptance");
            let mut seconds = [0; 2];
            rx.read_exact(&mut seconds).await?;
            let seconds = u16::from_be_bytes(seconds);
            ensure!(
                (1..=MAX_SECONDS).contains(&seconds),
                "Invalid host input duration"
            );
            Ok::<_, anyhow::Error>((tx, rx, seconds))
        })
        .await;
        let (tx, rx, seconds) = match handshake {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                connection.close(2u32.into(), b"input handshake failed");
                return Err(error);
            }
            Err(error) => {
                connection.close(2u32.into(), b"input handshake timed out");
                return Err(error.into());
            }
        };
        Ok(Self {
            connection,
            tx,
            rx,
            state: State::default(),
            deadline: Instant::now() + Duration::from_secs(u64::from(seconds)),
            closed: false,
        })
    }

    pub fn active(&self) -> bool {
        self.state.active()
    }

    /// A canceled future may leave bytes in flight. A caller must close the
    /// connection rather than retry a send whose acknowledgment was not read.
    pub async fn send(&mut self, event: Event) -> Result<()> {
        self.send_with_budget(event, IO_LIMIT).await
    }

    /// Limit a caller's remaining local event budget without changing the
    /// negotiated input/1 protocol. A failed acceptance is never retried.
    pub async fn send_with_budget(&mut self, event: Event, budget: Duration) -> Result<()> {
        ensure!(!self.closed, "Input connection closed");
        ensure!(!budget.is_zero(), "Input send budget expired");
        let record = self.state.next_record(event)?;
        let bytes = record.encode()?;
        let limit = remaining(self.deadline)?.min(budget);
        self.closed = true;
        let result = timeout(limit, async {
            self.tx.write_all(&bytes).await?;
            let mut acknowledgment = [0; 4];
            self.rx.read_exact(&mut acknowledgment).await?;
            ensure!(
                u32::from_be_bytes(acknowledgment) == record.sequence,
                "Input acknowledgment mismatch"
            );
            Ok::<(), anyhow::Error>(())
        })
        .await;
        match result {
            Ok(Ok(())) => {
                self.state.commit(record)?;
                if event == Event::End {
                    self.tx.finish()?;
                    self.connection.close(0u32.into(), b"input completed");
                } else {
                    self.closed = false;
                }
                Ok(())
            }
            Ok(Err(error)) => {
                self.connection.close(2u32.into(), b"input send failed");
                Err(error)
            }
            Err(error) => {
                self.connection.close(2u32.into(), b"input send timed out");
                Err(error.into())
            }
        }
    }
}

impl Drop for InputConnection {
    fn drop(&mut self) {
        self.connection
            .close(0u32.into(), b"input controller stopped");
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
