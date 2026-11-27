//! Explicitly approved, one-shot diagnostic sessions. No screen or input authority.
#![forbid(unsafe_code)]
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use intralink_transport::{bind, bind_controller, receive_probe_with_wait, send_probe};
use intralink_vault::{Direction, Peer, Profile, Route, Vault};
use iroh::{Endpoint, EndpointId, SecretKey};
use serde::Deserialize;
use tokio::{sync::watch, time::timeout};

const MAX_REQUEST_BYTES: usize = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairRequest {
    pub name: String,
    pub peer_id: String,
    pub direction: Direction,
    pub host: Option<String>,
    pub port: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeRequest {
    pub name: String,
    pub direction: Direction,
    pub host: Option<String>,
    pub port: Option<u16>,
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    ensure!(value.len() <= MAX_REQUEST_BYTES, "Request is too large");
    serde_json::from_str(value).context("Invalid diagnostic request")
}

fn private_address(host: &str, port: u16) -> Result<SocketAddr> {
    let address: Ipv4Addr = host.parse().context("Use a numeric private IPv4 address")?;
    ensure!(
        address.is_private() && port != 0,
        "Use a private IPv4 address and nonzero port"
    );
    Ok(SocketAddr::from((address, port)))
}

fn private_route(host: &str, port: u16) -> Result<Route> {
    private_address(host, port)?;
    Route::new(host, port)
}

/// The UI confirms the full independently checked public identity before this
/// operation. Pairing never starts networking or authorizes a remote capability.
pub(crate) fn pair(path: &Path, password: &[u8], request: PairRequest) -> Result<Profile> {
    let id: EndpointId = request
        .peer_id
        .parse()
        .context("Invalid peer public identity")?;
    ensure!(
        id.to_string() == request.peer_id,
        "Use the complete canonical public identity"
    );
    let route = match (request.direction, request.host, request.port) {
        (Direction::Inbound, None, None) => None,
        (Direction::Outbound, Some(host), Some(port)) => Some(private_route(&host, port)?),
        _ => bail!("Outbound pairing requires a private route; inbound pairing forbids one"),
    };
    let vault = Vault::lock(path)?;
    let mut profile = vault.load(password)?;
    profile.add_peer(Peer::new(&request.name, id, request.direction, route)?)?;
    vault.save(&profile, password)?;
    Ok(profile)
}

pub(crate) fn revoke(path: &Path, password: &[u8], name: &str) -> Result<Profile> {
    let vault = Vault::lock(path)?;
    let mut profile = vault.load(password)?;
    profile.revoke(name)?;
    vault.save(&profile, password)?;
    Ok(profile)
}

struct Active {
    id: i64,
    running: bool,
    stop: watch::Sender<bool>,
}

struct State {
    next: i64,
    active: Option<Active>,
}

/// A foreground session is reserved synchronously before the UI queues its
/// worker. Cancellation before claim prevents any subsequent socket bind.
/// A stale stop request can never cancel another session.
pub(crate) struct Registry(Mutex<State>);

impl Registry {
    pub const fn new() -> Self {
        Self(Mutex::new(State {
            next: 1,
            active: None,
        }))
    }

    pub fn reserve(&self) -> Result<i64> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Session registry unavailable"))?;
        ensure!(state.active.is_none(), "Another diagnostic is active");
        let id = state.next;
        state.next = id.checked_add(1).context("Session identifiers exhausted")?;
        let (stop, _) = watch::channel(false);
        state.active = Some(Active {
            id,
            running: false,
            stop,
        });
        Ok(id)
    }

    pub fn claim(&self, id: i64) -> Result<Lease<'_>> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Session registry unavailable"))?;
        let active = state
            .active
            .as_mut()
            .context("Diagnostic canceled or not reserved")?;
        ensure!(
            active.id == id && !active.running,
            "Invalid diagnostic reservation"
        );
        active.running = true;
        Ok(Lease {
            registry: self,
            id,
            stop: active.stop.subscribe(),
        })
    }

    pub fn cancel(&self, id: i64) {
        if let Ok(mut state) = self.0.lock()
            && let Some(active) = state.active.as_ref()
            && active.id == id
        {
            if active.running {
                active.stop.send_replace(true);
            } else {
                state.active = None;
            }
        }
    }
}

pub(crate) struct Lease<'a> {
    registry: &'a Registry,
    id: i64,
    stop: watch::Receiver<bool>,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.registry.0.lock()
            && state.active.as_ref().is_some_and(|a| a.id == self.id)
        {
            state.active = None;
        }
    }
}

async fn canceled(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|value| *value).await;
}

#[derive(Clone, Copy)]
enum Operation {
    Send {
        peer: EndpointId,
        target: SocketAddr,
    },
    Receive {
        peer: EndpointId,
        local: SocketAddr,
    },
}

fn operation(profile: &Profile, request: &ProbeRequest) -> Result<Operation> {
    let peer = profile.peer(&request.name, request.direction)?;
    match request.direction {
        Direction::Outbound => {
            ensure!(
                profile.role().can_initiate() && request.host.is_none() && request.port.is_none(),
                "BASE must use its saved outbound route"
            );
            let route = peer.route().context("Missing outbound route")?;
            Ok(Operation::Send {
                peer: peer.identity()?,
                target: private_address(route.host(), route.port())?,
            })
        }
        Direction::Inbound => {
            ensure!(profile.role().can_receive(), "CLIENT role required");
            let local = private_address(
                request
                    .host
                    .as_deref()
                    .context("Enter this device's private IPv4 address")?,
                request.port.context("Enter a listening port")?,
            )?;
            Ok(Operation::Receive {
                peer: peer.identity()?,
                local,
            })
        }
    }
}

async fn open(secret: SecretKey, operation: Operation) -> Result<Endpoint> {
    match operation {
        Operation::Send { target, .. } => bind_controller(secret, &[target]).await,
        Operation::Receive { local, .. } => bind(secret, local, true).await,
    }
}

async fn exchange(endpoint: &Endpoint, operation: Operation) -> Result<()> {
    match operation {
        Operation::Send { peer, target } => send_probe(endpoint, peer, target).await,
        Operation::Receive { peer, .. } => {
            receive_probe_with_wait(endpoint, peer, Duration::from_secs(120)).await
        }
    }
}

async fn execute(
    secret: SecretKey,
    operation: Operation,
    stop: &mut watch::Receiver<bool>,
) -> Result<()> {
    let endpoint = tokio::select! {
        biased;
        _ = canceled(stop) => bail!("Diagnostic canceled"),
        result = timeout(Duration::from_secs(15), open(secret, operation)) => result.context("Endpoint bind deadline exceeded")??,
    };
    let result = tokio::select! {
        biased;
        _ = canceled(stop) => Err(anyhow::anyhow!("Diagnostic canceled")),
        result = timeout(Duration::from_secs(150), exchange(&endpoint, operation)) => match result {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("Diagnostic deadline exceeded")),
        },
    };
    endpoint.close().await;
    result
}

/// Holds the vault lock for the whole approved diagnostic so the saved peer
/// cannot be replaced while in use. The password is not retained by networking.
pub(crate) fn run(
    mut lease: Lease<'_>,
    path: &Path,
    password: zeroize::Zeroizing<Vec<u8>>,
    request: ProbeRequest,
) -> Result<(Profile, u64)> {
    ensure!(!*lease.stop.borrow(), "Diagnostic canceled");
    let vault = Vault::lock(path)?;
    let profile = vault.load(&password)?;
    drop(password);
    let operation = operation(&profile, &request)?;
    ensure!(!*lease.stop.borrow(), "Diagnostic canceled");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let started = Instant::now();
    runtime.block_on(execute(profile.secret_key(), operation, &mut lease.stop))?;
    Ok((
        profile,
        started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    ))
}

/// A separate, ephemeral screen-only approval. A saved diagnostic grant is
/// necessary but not sufficient; the Activity reviews the exact identity and
/// obtains fresh OS consent before the worker invokes this entry point.
#[cfg(target_os = "android")]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScreenRequest {
    pub name: String,
    pub peer_id: String,
    pub host: String,
    pub port: u16,
    pub height: u16,
    pub fps: u32,
    #[serde(default = "preview_seconds")]
    pub seconds: u64,
}

#[cfg(target_os = "android")]
const fn preview_seconds() -> u64 {
    15
}

#[cfg(target_os = "android")]
pub(crate) fn run_screen(
    mut lease: Lease<'_>,
    path: &Path,
    password: zeroize::Zeroizing<Vec<u8>>,
    request: ScreenRequest,
    mut capture: impl FnMut(
        intralink_core::desktop::VideoConfig,
        intralink_core::quality::ReceiptPressure,
        Vec<u8>,
    ) -> Result<intralink_transport::desktop::Capture>,
) -> Result<(Profile, u64, intralink_transport::desktop::Stats)> {
    use intralink_core::{
        adaptive::ViewProtocol,
        desktop::{CodecMode, VideoConfig},
    };
    use intralink_transport::desktop::{
        CaptureRequest, bind_view_with_protocol, serve_view_with_protocol,
    };
    use tokio::sync::mpsc;

    ensure!(!*lease.stop.borrow(), "Screen approval canceled");
    let vault = Vault::lock(path)?;
    let profile = vault.load(&password)?;
    drop(password);
    ensure!(profile.role().can_receive(), "CLIENT role required");
    let peer = profile
        .peer(&request.name, Direction::Inbound)?
        .identity()?;
    ensure!(
        request.peer_id == peer.to_string(),
        "Approved identity changed"
    );
    let local = private_address(&request.host, request.port)?;
    ensure!(
        matches!(request.seconds, 15 | 90),
        "Unsupported local sharing duration"
    );
    let config = VideoConfig::new(request.height, request.fps, 75, request.seconds)
        .context("Invalid local screen limits")?
        .with_mode(CodecMode::Compact);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let started = Instant::now();
    let stats = runtime.block_on(async {
        let endpoint = tokio::select! {
            biased;
            _ = canceled(&mut lease.stop) => bail!("Screen approval canceled"),
            result = timeout(Duration::from_secs(10),
                bind_view_with_protocol(profile.secret_key(), local, ViewProtocol::Bounded)) =>
                    result.context("Screen endpoint bind deadline")??,
        };
        let (tx, mut rx) = mpsc::channel::<CaptureRequest>(1);
        let mut stop = lease.stop.clone();
        // Only one demand exists. The synchronous local adapter runs on the
        // attached JNI owner, never on a spawned Rust worker. Stop is checked
        // around every capture; an acquired image cannot escape that call.
        let source = async {
            while let Some(mut req) = rx.recv().await {
                ensure!(!*stop.borrow(), "Screen approval canceled");
                req.begin_capture()?;
                let chosen = req
                    .desktop_config
                    .context("Missing approved screen configuration")?;
                let result = capture(chosen, req.receipt_pressure, req.buffer);
                ensure!(!*stop.borrow_and_update(), "Screen approval canceled");
                let end = matches!(&result, Ok(intralink_transport::desktop::Capture::End));
                let failed = result.is_err();
                ensure!(req.reply.send(result).is_ok(), "Screen demand canceled");
                if failed {
                    bail!("Local screen source failed");
                }
                if end {
                    break;
                }
            }
            Ok::<(), anyhow::Error>(())
        };
        let result = tokio::select! {
            biased;
            _ = canceled(&mut lease.stop) => Err(anyhow::anyhow!("Screen approval canceled")),
            result = timeout(Duration::from_secs(request.seconds + 20), async {
                tokio::try_join!(
                    serve_view_with_protocol(&endpoint, peer, config, tx, ViewProtocol::Bounded),
                    source
                ).map(|(stats, ())| stats)
            }) => result.context("Screen session deadline").and_then(|result| result),
        };
        endpoint.close().await;
        result
    })?;
    Ok((
        profile,
        started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        stats,
    ))
}

#[cfg(all(target_os = "android", feature = "lan-fixture"))]
pub(crate) fn run_media_fixture(
    mut lease: Lease<'_>,
    path: &Path,
    password: zeroize::Zeroizing<Vec<u8>>,
    request: ProbeRequest,
) -> Result<(Profile, u64, intralink_transport::desktop::Stats)> {
    use intralink_transport::{desktop::bind_view_with_protocol, media_fixture};
    ensure!(!*lease.stop.borrow(), "Diagnostic canceled");
    let vault = Vault::lock(path)?;
    let profile = vault.load(&password)?;
    drop(password);
    let operation = operation(&profile, &request)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let started = Instant::now();
    let stats = runtime.block_on(async {
        let endpoint = tokio::select! {
            biased;
            _ = canceled(&mut lease.stop) => bail!("Fixture canceled"),
            result = timeout(Duration::from_secs(15), async {
                match operation {
                    Operation::Send { target, .. } => bind_controller(profile.secret_key(), &[target]).await,
                    Operation::Receive { local, .. } =>
                        bind_view_with_protocol(profile.secret_key(), local, media_fixture::PROTOCOL).await,
                }
            }) => result.context("Fixture bind deadline")??,
        };
        let result = tokio::select! {
            biased;
            _ = canceled(&mut lease.stop) => Err(anyhow::anyhow!("Fixture canceled")),
            result = timeout(Duration::from_secs(35), async {
                match operation {
                    Operation::Send { peer, target } => media_fixture::view(&endpoint, peer, target).await,
                    Operation::Receive { peer, .. } => media_fixture::serve(&endpoint, peer).await,
                }
            }) => result.context("Fixture session deadline").and_then(|result| result),
        };
        endpoint.close().await;
        result
    })?;
    Ok((
        profile,
        started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        stats,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use intralink_core::Role;
    use zeroize::Zeroizing;

    #[test]
    fn reservations_are_single_use_and_stale_cancellation_is_isolated() -> Result<()> {
        let registry = Registry::new();
        let first = registry.reserve()?;
        assert!(registry.reserve().is_err());
        registry.cancel(first);
        assert!(registry.claim(first).is_err());
        let second = registry.reserve()?;
        registry.cancel(first);
        let lease = registry.claim(second)?;
        assert!(registry.claim(second).is_err());
        assert!(!*lease.stop.borrow());
        registry.cancel(second);
        assert!(*lease.stop.borrow());
        assert!(registry.reserve().is_err());
        drop(lease);
        assert!(registry.reserve().is_ok());
        Ok(())
    }

    #[test]
    fn manual_pairing_checks_route_direction_and_identity_and_persists() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("profile.vault");
        let password = b"local test passphrase";
        let base = Profile::new(Role::Controller);
        let client = Profile::new(Role::Host);
        Vault::lock(&path)?.initialize(&base, password)?;
        let make = |host: &str, direction| PairRequest {
            name: "client".into(),
            peer_id: client.public_id().to_string(),
            direction,
            host: Some(host.into()),
            port: Some(45100),
        };
        assert!(pair(&path, password, make("example.com", Direction::Outbound)).is_err());
        assert!(pair(&path, password, make("8.8.8.8", Direction::Outbound)).is_err());
        assert!(pair(&path, password, make("127.0.0.1", Direction::Outbound)).is_err());
        assert!(pair(&path, password, make("192.168.1.2", Direction::Inbound)).is_err());
        let paired = pair(&path, password, make("192.168.1.2", Direction::Outbound))?;
        assert_eq!(paired.public_id(), base.public_id());
        assert_eq!(paired.peers().len(), 1);
        assert!(pair(&path, password, make("192.168.1.2", Direction::Outbound)).is_err());
        let request: ProbeRequest =
            decode(r#"{"name":"client","direction":"outbound","host":null,"port":null}"#)?;
        assert!(matches!(
            operation(&paired, &request)?,
            Operation::Send { .. }
        ));
        let invalid: ProbeRequest = decode(
            r#"{"name":"client","direction":"outbound","host":"192.168.1.3","port":45100}"#,
        )?;
        assert!(operation(&paired, &invalid).is_err());
        assert!(decode::<ProbeRequest>(&"x".repeat(MAX_REQUEST_BYTES + 1)).is_err());
        assert!(
            decode::<ProbeRequest>(r#"{"name":"client","direction":"outbound","unexpected":true}"#)
                .is_err()
        );
        assert!(revoke(&path, b"wrong test passphrase", "client").is_err());
        assert!(revoke(&path, password, "client")?.peers().is_empty());
        let registry = Registry::new();
        let id = registry.reserve()?;
        let lease = registry.claim(id)?;
        registry.cancel(id);
        assert!(run(lease, &path, Zeroizing::new(password.to_vec()), request).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn real_authenticated_quic_and_canceled_listener() -> Result<()> {
        // Loopback is test-only. The public Android entry point permits only
        // explicit private IPv4 addresses.
        let host = bind(SecretKey::generate(), "127.0.0.1:0".parse()?, true).await?;
        let client_secret = SecretKey::generate();
        let local = host.bound_sockets()[0];
        let peer = host.id();
        let client_id = client_secret.public();
        let registry = Registry::new();
        let id = registry.reserve()?;
        let mut lease = registry.claim(id)?;
        let (received, sent) = tokio::join!(
            receive_probe_with_wait(&host, client_id, Duration::from_secs(5)),
            execute(
                client_secret,
                Operation::Send {
                    peer,
                    target: local
                },
                &mut lease.stop
            )
        );
        host.close().await;
        received?;
        sent?;
        drop(lease);

        let id = registry.reserve()?;
        let mut lease = registry.claim(id)?;
        let (_, result) = tokio::join!(
            async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                registry.cancel(id);
            },
            execute(
                SecretKey::generate(),
                Operation::Receive {
                    peer,
                    local: "127.0.0.1:0".parse()?
                },
                &mut lease.stop
            )
        );
        assert!(result.is_err());
        drop(lease);
        assert!(registry.reserve().is_ok());
        Ok(())
    }
}
