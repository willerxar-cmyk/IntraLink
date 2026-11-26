//! Opt-in local IPv4 DNS-SD. TXT identifiers and addresses are untrusted hints.
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use anyhow::{Result, ensure};
use iroh::{EndpointId, SecretKey};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::time::{Instant, timeout};

const SERVICE: &str = "_intralink._udp.local.";
const BROWSE_LIMIT: Duration = Duration::from_secs(3);
const MAX_LAN_CANDIDATES: usize = 4;

pub fn eligible_address(addr: SocketAddr) -> bool {
    matches!(addr.ip(), IpAddr::V4(ip) if ip.is_private()) && addr.port() != 0
}

pub struct Advertisement {
    daemon: ServiceDaemon,
    name: String,
}

impl Advertisement {
    pub fn start(identity: EndpointId, addr: SocketAddr) -> Result<Self> {
        ensure!(
            eligible_address(addr),
            "LAN advertisement requires an explicit private IPv4 bind address"
        );
        let instance = format!("il-{}", SecretKey::generate().public().fmt_short());
        let host = format!("{instance}.local.");
        let properties = [
            ("id", identity.to_string()),
            ("protocol", "probe-1".to_owned()),
        ];
        let info = ServiceInfo::new(
            SERVICE,
            &instance,
            &host,
            addr.ip().to_string(),
            addr.port(),
            &properties[..],
        )?;
        let name = info.get_fullname().to_owned();
        let daemon = ServiceDaemon::new()?;
        if let Err(error) = daemon.register(info) {
            let _ = daemon.shutdown();
            return Err(error.into());
        }
        Ok(Self { daemon, name })
    }

    pub async fn stop(self) {
        if let Ok(status) = self.daemon.unregister(&self.name) {
            let _ = timeout(Duration::from_secs(2), status.recv_async()).await;
        }
        if let Ok(status) = self.daemon.shutdown() {
            let _ = timeout(Duration::from_secs(2), status.recv_async()).await;
        }
    }
}

pub async fn find(expected: EndpointId) -> Result<Vec<SocketAddr>> {
    let daemon = ServiceDaemon::new()?;
    let result = browse(&daemon, expected).await;
    let _ = daemon.stop_browse(SERVICE);
    if let Ok(status) = daemon.shutdown() {
        let _ = timeout(Duration::from_secs(2), status.recv_async()).await;
    }
    result
}

async fn browse(daemon: &ServiceDaemon, expected: EndpointId) -> Result<Vec<SocketAddr>> {
    let events = daemon.browse(SERVICE)?;
    let expected = expected.to_string();
    let deadline = Instant::now() + BROWSE_LIMIT;
    let mut candidates = Vec::new();
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        let event = match timeout(remaining, events.recv_async()).await {
            Ok(Ok(event)) => event,
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => break,
        };
        if let ServiceEvent::ServiceResolved(service) = event {
            if service.get_property_val_str("id") != Some(expected.as_str())
                || service.get_property_val_str("protocol") != Some("probe-1")
            {
                continue;
            }
            for ip in service.get_addresses_v4() {
                let addr = SocketAddr::from((ip.to_owned(), service.get_port()));
                if eligible_address(addr) && !candidates.contains(&addr) {
                    candidates.push(addr);
                    if candidates.len() == MAX_LAN_CANDIDATES {
                        return Ok(candidates);
                    }
                }
            }
        }
    }
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_is_restricted_to_private_ipv4_hints() -> Result<()> {
        for addr in ["10.1.1.1:45100", "192.168.1.2:45100", "172.31.0.1:45100"] {
            assert!(eligible_address(addr.parse()?));
        }
        for addr in [
            "127.0.0.1:45100",
            "0.0.0.0:45100",
            "100.64.0.1:45100",
            "192.168.1.2:0",
            "[fd00::1]:45100",
        ] {
            assert!(!eligible_address(addr.parse()?));
        }
        Ok(())
    }
}
