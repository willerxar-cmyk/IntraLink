//! Direct-only authenticated transports. No unattended or privileged operations.
#![forbid(unsafe_code)]

use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use intralink_core::{ALPN, MAX_PROBE_BYTES, PROBE, REPLY, is_probe};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey,
    endpoint::{BindOpts, presets},
};
use tokio::time::timeout;

pub mod desktop;
pub mod input;
pub mod input_queue;

use intralink_core::network::{MAX_CANDIDATES, select_candidates};

const HANDSHAKE_LIMIT: Duration = Duration::from_secs(10);
const SESSION_LIMIT: Duration = Duration::from_secs(20);

/// Bind a single explicitly chosen socket. No relay, directory or default discovery.
pub async fn bind(secret: SecretKey, addr: SocketAddr, receive: bool) -> Result<Endpoint> {
    let protocol = receive.then_some(ALPN);
    bind_for_protocol(secret, addr, protocol).await
}

/// Only enable the explicitly selected protocol on a receiving endpoint.
pub(crate) async fn bind_for_protocol(
    secret: SecretKey,
    addr: SocketAddr,
    protocol: Option<&[u8]>,
) -> Result<Endpoint> {
    let alpns = protocol.into_iter().map(|p| p.to_vec()).collect();
    Ok(Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .clear_relay_transports()
        .clear_address_lookup()
        .clear_ip_transports()
        .secret_key(secret)
        .alpns(alpns)
        .bind_addr(addr)?
        .bind()
        .await?)
}

/// One manually authorized session, then stop listening. The remote TLS identity
/// must match the key verified by the local operator before any protocol data.
pub async fn receive_probe(endpoint: &Endpoint, authorized: EndpointId) -> Result<()> {
    receive_probe_with_wait(endpoint, authorized, SESSION_LIMIT).await
}

/// A longer, bounded foreground acceptance window for interactive operators.
pub async fn receive_probe_with_wait(
    endpoint: &Endpoint,
    authorized: EndpointId,
    wait: Duration,
) -> Result<()> {
    ensure!(
        !wait.is_zero() && wait <= Duration::from_secs(120),
        "Invalid listener wait interval"
    );
    let incoming = timeout(wait, endpoint.accept())
        .await
        .context("No incoming connection before deadline")?
        .context("Endpoint closed")?;
    let connection = timeout(HANDSHAKE_LIMIT, async { incoming.await }).await??;
    if connection.remote_id() != authorized {
        connection.close(1u32.into(), b"not authorized");
        bail!("Unrecognized peer; connection rejected");
    }
    let result = timeout(SESSION_LIMIT, async {
        let (mut tx, mut rx) = connection.accept_bi().await?;
        let data = rx.read_to_end(MAX_PROBE_BYTES).await?;
        ensure!(is_probe(&data), "Unsupported protocol message");
        tx.write_all(REPLY).await?;
        tx.finish()?;
        // The application response must arrive before closing the connection.
        let _ = connection.closed().await;
        Ok::<(), anyhow::Error>(())
    })
    .await;
    if result.is_err() || result.as_ref().is_ok_and(|r| r.is_err()) {
        connection.close(2u32.into(), b"probe failed");
    }
    result.context("Probe session deadline exceeded")??;
    Ok(())
}

/// DNS selects a route only; the pinned endpoint ID authenticates the host.
pub async fn send_probe(
    endpoint: &Endpoint,
    expected: EndpointId,
    remote: SocketAddr,
) -> Result<()> {
    send_probe_candidates(endpoint, expected, &[remote]).await
}

/// Apply the same bounded routing policy even when called without the CLI.
fn direct_candidates(candidates: &[SocketAddr]) -> Result<Vec<SocketAddr>> {
    ensure!(
        !candidates.is_empty() && candidates.len() <= MAX_CANDIDATES,
        "Expected 1-8 routing candidates"
    );
    let selected = select_candidates(candidates.iter().copied());
    ensure!(!selected.is_empty(), "No usable direct routing candidates");
    Ok(selected)
}

/// Bind only the address families needed by the explicit routing candidates.
/// IPv6 is optional when IPv4 is also available; no relay is introduced.
pub async fn bind_controller(secret: SecretKey, candidates: &[SocketAddr]) -> Result<Endpoint> {
    let candidates = direct_candidates(candidates)?;
    let v4 = candidates.iter().any(SocketAddr::is_ipv4);
    let v6 = candidates.iter().any(SocketAddr::is_ipv6);
    bind_controller_families(secret, v4, v6, false).await
}

/// A workspace keeps one outbound-only endpoint across independently authorized
/// connections and later DNS resolutions. Bind both available IP families up front;
/// an unavailable family must not prevent using the other.
pub async fn bind_workspace_controller(secret: SecretKey) -> Result<Endpoint> {
    bind_controller_families(secret, true, true, true).await
}

async fn bind_controller_families(
    secret: SecretKey,
    v4: bool,
    v6: bool,
    optional: bool,
) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .clear_relay_transports()
        .clear_address_lookup()
        .clear_ip_transports()
        .secret_key(secret);
    if v4 {
        builder = builder
            .bind_addr_with_opts("0.0.0.0:0", BindOpts::default().set_is_required(!optional))?;
    }
    if v6 {
        builder = builder.bind_addr_with_opts(
            "[::]:0",
            BindOpts::default().set_is_required(!v4 && !optional),
        )?;
    }
    let endpoint = builder.bind().await?;
    if endpoint.bound_sockets().is_empty() {
        endpoint.close().await;
        bail!("No local IP transport available");
    }
    Ok(endpoint)
}

/// The pinned identity remains the same for every candidate. Iroh chooses a direct path.
pub async fn send_probe_candidates(
    endpoint: &Endpoint,
    expected: EndpointId,
    candidates: &[SocketAddr],
) -> Result<()> {
    let candidates = direct_candidates(candidates)?;
    let target = candidates
        .iter()
        .fold(EndpointAddr::new(expected), |target, addr| {
            target.with_ip_addr(*addr)
        });
    let connection = timeout(HANDSHAKE_LIMIT, endpoint.connect(target, ALPN)).await??;
    ensure!(
        connection.remote_id() == expected,
        "Remote identity mismatch"
    );
    let result = timeout(SESSION_LIMIT, async {
        let (mut tx, mut rx) = connection.open_bi().await?;
        tx.write_all(PROBE).await?;
        tx.finish()?;
        let data = rx.read_to_end(MAX_PROBE_BYTES).await?;
        ensure!(data == REPLY, "Unexpected probe response");
        Ok::<(), anyhow::Error>(())
    })
    .await;
    let success = result.as_ref().is_ok_and(|r| r.is_ok());
    if success {
        connection.close(0u32.into(), b"probe complete");
    } else {
        connection.close(2u32.into(), b"probe failed");
    }
    result.context("Probe session deadline exceeded")??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn authorized_probe_on_loopback() -> Result<()> {
        let host = bind(SecretKey::generate(), "127.0.0.1:0".parse()?, true).await?;
        let controller = bind(SecretKey::generate(), "127.0.0.1:0".parse()?, false).await?;
        let addr = host
            .bound_sockets()
            .into_iter()
            .find(|a| a.is_ipv4())
            .context("Missing loopback address")?;
        let (received, sent) = tokio::join!(
            receive_probe(&host, controller.id()),
            send_probe(&controller, host.id(), addr),
        );
        controller.close().await;
        host.close().await;
        received?;
        sent?;
        Ok(())
    }

    #[tokio::test]
    async fn multiple_candidates_include_reachable_address() -> Result<()> {
        let host = bind(SecretKey::generate(), "127.0.0.1:0".parse()?, true).await?;
        let addr = host
            .bound_sockets()
            .into_iter()
            .find(|a| a.is_ipv4())
            .context("Missing loopback address")?;
        let candidates = [addr, "[::1]:1".parse()?];
        let controller = bind_controller(SecretKey::generate(), &candidates).await?;
        let (received, sent) = tokio::join!(
            receive_probe(&host, controller.id()),
            send_probe_candidates(&controller, host.id(), &candidates),
        );
        controller.close().await;
        host.close().await;
        received?;
        sent?;
        Ok(())
    }

    #[tokio::test]
    async fn mapped_ipv4_route_uses_real_ipv4_transport() -> Result<()> {
        let host = bind(SecretKey::generate(), "127.0.0.1:0".parse()?, true).await?;
        let addr = host
            .bound_sockets()
            .into_iter()
            .find(|a| a.is_ipv4())
            .context("Missing IPv4 loopback address")?;
        let SocketAddr::V4(v4) = addr else {
            bail!("Expected IPv4 socket");
        };
        let mapped = SocketAddr::new(v4.ip().to_ipv6_mapped().into(), v4.port());
        let controller = bind_controller(SecretKey::generate(), &[mapped]).await?;
        let candidates = [mapped, addr];
        let (received, sent) = tokio::join!(
            receive_probe(&host, controller.id()),
            send_probe_candidates(&controller, host.id(), &candidates),
        );
        controller.close().await;
        host.close().await;
        received?;
        sent?;
        Ok(())
    }

    #[tokio::test]
    async fn wrong_host_pin_is_rejected() -> Result<()> {
        let host = bind(SecretKey::generate(), "127.0.0.1:0".parse()?, true).await?;
        let controller = bind(SecretKey::generate(), "127.0.0.1:0".parse()?, false).await?;
        let addr = host
            .bound_sockets()
            .into_iter()
            .find(|a| a.is_ipv4())
            .context("Missing loopback address")?;
        let unrelated = SecretKey::generate().public();
        let (received, sent) = tokio::join!(
            receive_probe(&host, controller.id()),
            send_probe(&controller, unrelated, addr),
        );
        controller.close().await;
        host.close().await;
        assert!(sent.is_err());
        assert!(received.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn unapproved_identity_is_rejected() -> Result<()> {
        let host = bind(SecretKey::generate(), "127.0.0.1:0".parse()?, true).await?;
        let controller = bind(SecretKey::generate(), "127.0.0.1:0".parse()?, false).await?;
        let stranger = SecretKey::generate().public();
        let addr = host
            .bound_sockets()
            .into_iter()
            .find(|a| a.is_ipv4())
            .context("Missing loopback address")?;
        let (received, sent) = tokio::join!(
            receive_probe(&host, stranger),
            send_probe(&controller, host.id(), addr),
        );
        controller.close().await;
        host.close().await;
        assert!(received.is_err());
        assert!(sent.is_err());
        Ok(())
    }
}
