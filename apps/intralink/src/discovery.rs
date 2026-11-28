//! Opt-in local IPv4 DNS-SD. TXT identifiers and addresses are untrusted hints.
use std::{
    collections::BTreeMap,
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
const MAX_PEERS: usize = 128;
const MAX_EVENTS: usize = 1024;
const MAX_ADDRESSES: usize = 32;

#[derive(Clone, Copy)]
pub enum Protocol {
    Probe,
    #[cfg(any(
        all(feature = "input-preview", target_os = "windows"),
        all(feature = "input-x11-preview", target_os = "linux")
    ))]
    Input,
    #[cfg(all(feature = "input-preview", target_os = "windows"))]
    InputGeometry,
}

impl Protocol {
    fn label(self) -> &'static str {
        match self {
            Self::Probe => "probe-1",
            #[cfg(any(
                all(feature = "input-preview", target_os = "windows"),
                all(feature = "input-x11-preview", target_os = "linux")
            ))]
            Self::Input => "input-1",
            #[cfg(all(feature = "input-preview", target_os = "windows"))]
            Self::InputGeometry => "input-2",
        }
    }
}

pub fn eligible_address(addr: SocketAddr) -> bool {
    matches!(addr.ip(), IpAddr::V4(ip) if ip.is_private()) && addr.port() != 0
}

pub struct Advertisement {
    daemon: ServiceDaemon,
    name: String,
}

impl Advertisement {
    pub fn start(identity: EndpointId, addr: SocketAddr) -> Result<Self> {
        Self::for_protocol(identity, addr, Protocol::Probe)
    }

    pub fn for_protocol(
        identity: EndpointId,
        addr: SocketAddr,
        protocol: Protocol,
    ) -> Result<Self> {
        ensure!(
            eligible_address(addr),
            "LAN advertisement requires an explicit private IPv4 bind address"
        );
        let instance = format!("il-{}", SecretKey::generate().public().fmt_short());
        let host = format!("{instance}.local.");
        let properties = [
            ("id", identity.to_string()),
            ("protocol", protocol.label().to_owned()),
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

pub type Hints = BTreeMap<EndpointId, Vec<SocketAddr>>;

struct Collector {
    expected: BTreeMap<String, EndpointId>,
    hints: Hints,
    protocol: Protocol,
}

impl Collector {
    fn new(expected: &[EndpointId], protocol: Protocol) -> Result<Self> {
        ensure!(
            (1..=MAX_PEERS).contains(&expected.len()),
            "Select 1-128 known discovery identities"
        );
        let keys: BTreeMap<_, _> = expected.iter().map(|id| (id.to_string(), *id)).collect();
        ensure!(keys.len() == expected.len(), "Duplicate discovery identity");
        Ok(Self {
            expected: keys,
            hints: Hints::new(),
            protocol,
        })
    }

    fn collect(
        &mut self,
        id: Option<&str>,
        protocol: Option<&str>,
        addresses: impl Iterator<Item = SocketAddr>,
    ) {
        if protocol != Some(self.protocol.label()) {
            return;
        }
        let Some(known) = id.and_then(|id| self.expected.get(id)).copied() else {
            return;
        };
        for address in addresses.take(MAX_ADDRESSES) {
            if !eligible_address(address) {
                continue;
            }
            let candidates = self.hints.entry(known).or_default();
            if !candidates.contains(&address) && candidates.len() < MAX_LAN_CANDIDATES {
                candidates.push(address);
            }
            if candidates.len() == MAX_LAN_CANDIDATES {
                break;
            }
        }
    }

    fn full(&self) -> bool {
        self.hints.len() == self.expected.len()
            && self
                .hints
                .values()
                .all(|rows| rows.len() == MAX_LAN_CANDIDATES)
    }
}

pub async fn find(expected: EndpointId) -> Result<Vec<SocketAddr>> {
    Ok(find_many(&[expected], Protocol::Probe)
        .await?
        .remove(&expected)
        .unwrap_or_default())
}

/// One finite browse for the whole approved peer set. Never report an
/// unknown identity and never turn an announcement into a session grant.
pub async fn find_many(expected: &[EndpointId], protocol: Protocol) -> Result<Hints> {
    let mut collector = Collector::new(expected, protocol)?;
    let daemon = ServiceDaemon::new()?;
    let result = browse(&daemon, &mut collector).await;
    let _ = daemon.stop_browse(SERVICE);
    if let Ok(status) = daemon.shutdown() {
        let _ = timeout(Duration::from_secs(2), status.recv_async()).await;
    }
    result?;
    Ok(collector.hints)
}

async fn browse(daemon: &ServiceDaemon, collector: &mut Collector) -> Result<()> {
    let events = daemon.browse(SERVICE)?;
    let deadline = Instant::now() + BROWSE_LIMIT;
    for _ in 0..MAX_EVENTS {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        let event = match timeout(remaining, events.recv_async()).await {
            Ok(Ok(event)) => event,
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => break,
        };
        if let ServiceEvent::ServiceResolved(service) = event {
            collector.collect(
                service.get_property_val_str("id"),
                service.get_property_val_str("protocol"),
                service
                    .get_addresses_v4()
                    .iter()
                    .map(|ip| SocketAddr::from((*ip, service.get_port()))),
            );
            if collector.full() {
                break;
            }
        }
    }
    Ok(())
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

    #[tokio::test]
    #[ignore = "explicit native read-only LAN browse; no listener or advertisement"]
    async fn native_discovery_browse_generated_identity() -> Result<()> {
        let id = SecretKey::generate().public();
        let start = Instant::now();
        let found = find_many(&[id], Protocol::Probe).await?;
        ensure!(found.keys().all(|key| *key == id), "Unexpected identity");
        ensure!(
            found.is_empty(),
            "Generated identity unexpectedly announced"
        );
        println!(
            "NATIVE_LAN_BROWSE_PASS elapsed_ms={} matched=0",
            start.elapsed().as_millis()
        );
        Ok(())
    }

    #[test]
    fn batch_discovery_rejects_unknown_protocol_and_bounds_each_identity() -> Result<()> {
        let a = SecretKey::from_bytes(&[1; 32]).public();
        let b = SecretKey::from_bytes(&[2; 32]).public();
        let unknown = SecretKey::from_bytes(&[3; 32]).public();
        assert!(Collector::new(&[], Protocol::Probe).is_err());
        assert!(Collector::new(&[a, a], Protocol::Probe).is_err());
        let mut collector = Collector::new(&[a, b], Protocol::Probe)?;
        let addr: SocketAddr = "192.168.1.2:45100".parse()?;
        collector.collect(
            Some(&unknown.to_string()),
            Some("probe-1"),
            [addr].into_iter(),
        );
        collector.collect(Some(&a.to_string()), Some("input-1"), [addr].into_iter());
        assert!(collector.hints.is_empty());
        collector.collect(
            Some(&a.to_string()),
            Some("probe-1"),
            [addr, addr, "8.8.8.8:53".parse()?].into_iter(),
        );
        for i in 3..20 {
            collector.collect(
                Some(&a.to_string()),
                Some("probe-1"),
                [SocketAddr::from(([192, 168, 1, i], 45100))].into_iter(),
            );
        }
        collector.collect(Some(&b.to_string()), Some("probe-1"), [addr].into_iter());
        assert_eq!(collector.hints[&a].len(), 4);
        assert_eq!(collector.hints[&b], vec![addr]);
        assert!(!collector.full());
        assert!(!collector.hints.contains_key(&unknown));
        Ok(())
    }

    #[cfg(any(
        all(feature = "input-preview", target_os = "windows"),
        all(feature = "input-x11-preview", target_os = "linux")
    ))]
    #[test]
    fn input_announcements_are_separate_from_probe() -> Result<()> {
        let id = SecretKey::from_bytes(&[4; 32]).public();
        let mut collector = Collector::new(&[id], Protocol::Input)?;
        let addr = "10.0.0.1:45100".parse()?;
        collector.collect(Some(&id.to_string()), Some("probe-1"), [addr].into_iter());
        assert!(collector.hints.is_empty());
        collector.collect(Some(&id.to_string()), Some("input-1"), [addr].into_iter());
        assert_eq!(collector.hints[&id], vec![addr]);
        Ok(())
    }
}

#[cfg(all(test, target_os = "windows", feature = "input-preview"))]
#[test]
fn geometry_discovery_is_capability_specific_and_never_uses_input_1() -> Result<()> {
    let id = SecretKey::from_bytes(&[5; 32]).public();
    let text = id.to_string();
    let mut c = Collector::new(&[id], Protocol::InputGeometry)?;
    let address: SocketAddr = "192.168.1.3:45100".parse()?;
    c.collect(Some(&text), Some("input-1"), [address].into_iter());
    assert!(c.hints.is_empty());
    c.collect(Some(&text), Some("input-2"), [address].into_iter());
    assert_eq!(c.hints[&id], vec![address]);
    Ok(())
}
