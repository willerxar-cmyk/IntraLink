//! Bounded DNS resolution; routing data never grants access to a peer.
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use intralink_vault::Route;
use tokio::{net::lookup_host, time::timeout};

use intralink_core::network::select_candidates;
pub use intralink_core::network::{
    WanComparison, address_class, compare_wan, is_shared_v4, normalize_ip,
};

pub async fn resolve(route: &Route) -> Result<Vec<SocketAddr>> {
    let candidates = if let Ok(ip) = route.host().parse::<IpAddr>() {
        select_candidates([SocketAddr::new(ip, route.port())])
    } else {
        let addresses = timeout(
            Duration::from_secs(5),
            lookup_host((route.host(), route.port())),
        )
        .await
        .context("DNS lookup timed out")??;
        select_candidates(addresses)
    };
    ensure!(
        !candidates.is_empty(),
        "No usable direct IP address was resolved; interface-scoped IPv6 routes are unsupported"
    );
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_classes_do_not_claim_reachability() -> Result<()> {
        assert!(is_shared_v4("100.64.0.0".parse()?));
        assert!(is_shared_v4("100.127.255.255".parse()?));
        assert!(!is_shared_v4("100.63.255.255".parse()?));
        assert!(!is_shared_v4("100.128.0.0".parse()?));
        assert_eq!(address_class("192.168.1.1".parse()?), "private IPv4");
        assert_eq!(address_class("fd00::1".parse()?), "unique-local IPv6");
        Ok(())
    }

    #[tokio::test]
    async fn numeric_routes_are_normalized_without_dns() -> Result<()> {
        let route = Route::new("::ffff:127.0.0.1", 45100)?;
        assert_eq!(
            resolve(&route).await?,
            ["127.0.0.1:45100".parse::<SocketAddr>()?]
        );
        let invalid = Route::new("::ffff:0.0.0.0", 45100)?;
        assert!(resolve(&invalid).await.is_err());
        Ok(())
    }

    #[test]
    fn candidates_are_bounded_unique_and_dual_stack() -> Result<()> {
        let mut input: Vec<SocketAddr> = (1..=20)
            .map(|n| format!("192.0.2.{n}:45100").parse())
            .collect::<std::result::Result<_, _>>()?;
        input.push(input[0]);
        input.push("[2001:db8::1]:45100".parse()?);
        input.push("0.0.0.0:45100".parse()?);
        input.push("255.255.255.255:45100".parse()?);
        let result = select_candidates(input);
        assert_eq!(result.len(), 5);
        assert_eq!(result.iter().filter(|a| a.is_ipv6()).count(), 1);
        assert_eq!(result.iter().filter(|a| a.is_ipv4()).count(), 4);
        Ok(())
    }
}
