//! Platform-neutral routing policy. Addresses are untrusted hints, never identities.
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub const MAX_PER_FAMILY: usize = 4;
pub const MAX_CANDIDATES: usize = MAX_PER_FAMILY * 2;
const MAX_DNS_ANSWERS: usize = 256;

/// IPv4-mapped IPv6 addresses must use the same route and family as native IPv4.
pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

/// RFC 6598 shared address space is a clue, not proof of carrier NAT.
pub fn is_shared_v4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a == 100 && (64..=127).contains(&b)
}

pub fn address_class(ip: IpAddr) -> &'static str {
    let ip = normalize_ip(ip);
    if ip.is_unspecified() {
        return "unspecified";
    }
    if ip.is_loopback() {
        return "loopback";
    }
    if ip.is_multicast() {
        return "multicast";
    }
    match ip {
        IpAddr::V4(v4) if is_shared_v4(v4) => "RFC 6598 shared IPv4",
        IpAddr::V4(v4) if v4.is_private() => "private IPv4",
        IpAddr::V4(v4) if v4.is_link_local() => "link-local IPv4",
        IpAddr::V4(v4) if v4.is_documentation() => "documentation IPv4",
        IpAddr::V4(v4) if v4.is_broadcast() => "broadcast IPv4",
        IpAddr::V4(_) => "other IPv4 (reachability unknown)",
        IpAddr::V6(v6) if v6.is_unique_local() => "unique-local IPv6",
        IpAddr::V6(v6) if v6.is_unicast_link_local() => "link-local IPv6",
        IpAddr::V6(v6) if v6.segments()[..2] == [0x2001, 0x0db8] => "documentation IPv6",
        IpAddr::V6(_) => "other IPv6 (reachability unknown)",
    }
}

/// Direct addressing currently supports no interface-specific IPv6 routes.
/// Do not silently discard a zone identifier or guess the interface.
fn canonical_candidate(addr: SocketAddr) -> Option<SocketAddr> {
    if addr.port() == 0 {
        return None;
    }
    if matches!(addr, SocketAddr::V6(v6) if v6.scope_id() != 0 || v6.ip().is_unicast_link_local()) {
        return None;
    }
    let ip = normalize_ip(addr.ip());
    if ip.is_unspecified() || ip.is_multicast() {
        return None;
    }
    if matches!(ip, IpAddr::V4(v4) if v4.is_broadcast() || v4.octets()[0] == 0) {
        return None;
    }
    // Flow labels are not part of a saved endpoint route.
    Some(SocketAddr::new(ip, addr.port()))
}

/// Bounded deduplication with at most four entries per address family.
/// Inspect at most 256 answers; preserve resolver order within each family.
/// The transport decides which authenticated path actually works.
pub fn select_candidates(addresses: impl IntoIterator<Item = SocketAddr>) -> Vec<SocketAddr> {
    let mut v4 = Vec::with_capacity(MAX_PER_FAMILY);
    let mut v6 = Vec::with_capacity(MAX_CANDIDATES);
    for address in addresses.into_iter().take(MAX_DNS_ANSWERS) {
        let Some(address) = canonical_candidate(address) else {
            continue;
        };
        let family = if address.is_ipv4() { &mut v4 } else { &mut v6 };
        if family.len() < MAX_PER_FAMILY && !family.contains(&address) {
            family.push(address);
        }
        if v4.len() == MAX_PER_FAMILY && v6.len() == MAX_PER_FAMILY {
            break;
        }
    }
    v6.extend(v4);
    v6
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WanComparison {
    Match,
    Different,
    NoSameFamily,
    InvalidWan,
}

/// Compare an owner-supplied WAN address with the saved host's DNS/IP candidates.
/// This is not a public-IP lookup, NAT classification or connectivity test.
pub fn compare_wan(wan: IpAddr, candidates: &[SocketAddr]) -> WanComparison {
    let Some(wan) = canonical_candidate(SocketAddr::new(wan, 1)) else {
        return WanComparison::InvalidWan;
    };
    if wan.ip().is_loopback() || matches!(wan.ip(), IpAddr::V4(v4) if v4.is_link_local()) {
        return WanComparison::InvalidWan;
    }
    let mut same_family = false;
    for candidate in candidates.iter().copied().filter_map(canonical_candidate) {
        if candidate.is_ipv4() == wan.is_ipv4() {
            if candidate.ip() == wan.ip() {
                return WanComparison::Match;
            }
            same_family = true;
        }
    }
    if same_family {
        WanComparison::Different
    } else {
        WanComparison::NoSameFamily
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn mapped_addresses_are_deduplicated_before_family_limits() {
        let input = [
            addr("[::ffff:192.168.1.2]:45100"),
            addr("192.168.1.2:45100"),
            addr("[2001:db8::1]:45100"),
            addr("192.168.1.3:45100"),
        ];
        assert_eq!(select_candidates(input), [input[2], input[1], input[3]]);
    }

    #[test]
    fn malformed_or_unsupported_destinations_are_not_attempted() {
        let invalid = [
            addr("0.0.0.0:45100"),
            addr("[::]:45100"),
            addr("255.255.255.255:45100"),
            addr("224.0.0.1:45100"),
            addr("[ff02::1]:45100"),
            addr("[::ffff:0.0.0.0]:45100"),
            addr("[::ffff:224.0.0.1]:45100"),
            addr("[::ffff:255.255.255.255]:45100"),
            addr("0.1.2.3:45100"),
            addr("127.0.0.1:0"),
            addr("[fe80::1]:45100"),
            addr("[fe80::1%2]:45100"),
        ];
        assert!(select_candidates(invalid).is_empty());
        let scoped_global = SocketAddr::V6(std::net::SocketAddrV6::new(
            "2001:db8::1".parse().unwrap(),
            45100,
            0,
            2,
        ));
        assert!(select_candidates([scoped_global]).is_empty());
    }

    #[test]
    fn each_family_is_bounded_and_selection_is_idempotent() {
        let input = (1..=16).flat_map(|n| {
            [
                addr(&format!("192.0.2.{n}:45100")),
                addr(&format!("[2001:db8::{n}]:45100")),
            ]
        });
        let selected = select_candidates(input);
        assert_eq!(selected.len(), MAX_CANDIDATES);
        assert_eq!(
            selected.iter().filter(|a| a.is_ipv4()).count(),
            MAX_PER_FAMILY
        );
        assert_eq!(
            selected.iter().filter(|a| a.is_ipv6()).count(),
            MAX_PER_FAMILY
        );
        assert_eq!(select_candidates(selected.iter().copied()), selected);
    }

    #[test]
    fn scan_budget_is_bounded_even_for_duplicate_answers() {
        let repeated = std::iter::repeat_n(addr("192.0.2.1:45100"), MAX_DNS_ANSWERS)
            .chain([addr("[2001:db8::1]:45100")]);
        assert_eq!(select_candidates(repeated), [addr("192.0.2.1:45100")]);
    }

    #[test]
    fn comparison_is_family_aware_and_normalizes_mapped_values() {
        let candidates = [addr("192.0.2.1:45100"), addr("[2001:db8::1]:45100")];
        assert_eq!(
            compare_wan("::ffff:192.0.2.1".parse().unwrap(), &candidates),
            WanComparison::Match
        );
        assert_eq!(
            compare_wan("192.0.2.2".parse().unwrap(), &candidates),
            WanComparison::Different
        );
        assert_eq!(
            compare_wan("2001:db8::2".parse().unwrap(), &candidates[..1]),
            WanComparison::NoSameFamily
        );
        assert_eq!(
            compare_wan("127.0.0.1".parse().unwrap(), &candidates),
            WanComparison::InvalidWan
        );
        assert_eq!(
            compare_wan("169.254.1.2".parse().unwrap(), &candidates),
            WanComparison::InvalidWan
        );
        assert_eq!(
            compare_wan("::ffff:0.0.0.0".parse().unwrap(), &candidates),
            WanComparison::InvalidWan
        );
        assert_eq!(
            compare_wan("192.0.2.1".parse().unwrap(), &[]),
            WanComparison::NoSameFamily
        );
    }

    #[test]
    fn private_and_shared_ip_labels_do_not_assert_reachability() {
        for s in ["100.64.0.0", "100.127.255.255", "::ffff:100.64.1.1"] {
            assert_eq!(address_class(s.parse().unwrap()), "RFC 6598 shared IPv4");
        }
        for s in ["100.63.255.255", "100.128.0.0"] {
            assert_eq!(
                address_class(s.parse().unwrap()),
                "other IPv4 (reachability unknown)"
            );
        }
        assert_eq!(
            address_class("::ffff:192.168.1.2".parse().unwrap()),
            "private IPv4"
        );
        assert_eq!(
            address_class("fd00::1".parse().unwrap()),
            "unique-local IPv6"
        );
        assert_eq!(
            address_class("::ffff:127.0.0.1".parse().unwrap()),
            "loopback"
        );
    }
}
