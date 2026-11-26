//! Small, platform-neutral protocol and policy primitives.
#![forbid(unsafe_code)]

pub mod adaptive;
pub mod calibration;
pub mod canvas;
pub mod capabilities;
pub mod desktop;
pub mod h264;
pub mod journal;
pub mod media;
pub mod network;
pub mod overview;
pub mod pacing;
pub mod presentation;
pub mod profile_limits;
pub mod quality;
pub mod resources;
pub mod telemetry;
pub mod workspace;
pub mod yuv;

pub const ALPN: &[u8] = b"intralink/probe/1";
pub const PROBE: &[u8] = b"INTRALINK/1 PING";
pub const REPLY: &[u8] = b"INTRALINK/1 PONG";
pub const MAX_PROBE_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Controller,
    Host,
    Hybrid,
}

impl Role {
    pub const fn can_initiate(self) -> bool {
        matches!(self, Self::Controller | Self::Hybrid)
    }

    pub const fn can_receive(self) -> bool {
        matches!(self, Self::Host | Self::Hybrid)
    }
}

/// Only an exact, versioned probe is recognized; no generic commands or payloads.
pub fn is_probe(data: &[u8]) -> bool {
    data == PROBE
}

/// Validate one previously registered DuckDNS label. No account-wide updates.
pub fn duckdns_label(input: &str) -> Option<String> {
    let domain = input.to_ascii_lowercase();
    let label = domain.strip_suffix(".duckdns.org").unwrap_or(&domain);
    if label.is_empty()
        || label.len() > 63
        || label.starts_with('-')
        || label.ends_with('-')
        || !label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return None;
    }
    Some(label.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_are_directional() {
        assert!(Role::Controller.can_initiate());
        assert!(!Role::Controller.can_receive());
        assert!(Role::Host.can_receive());
        assert!(!Role::Host.can_initiate());
        assert!(Role::Hybrid.can_receive() && Role::Hybrid.can_initiate());
    }

    #[test]
    fn only_exact_probe_is_valid() {
        assert!(is_probe(PROBE));
        assert!(!is_probe(b"INTRALINK/1 PING\n"));
        assert!(!is_probe(b"INTRALINK/2 PING"));
        assert!(!is_probe(b""));
    }

    #[test]
    fn accepts_only_single_duckdns_label() {
        assert_eq!(duckdns_label("My-PC.duckdns.org").as_deref(), Some("my-pc"));
        assert_eq!(duckdns_label("pc-02").as_deref(), Some("pc-02"));
        for bad in [
            "",
            "-a",
            "a-",
            "a,b",
            "a.b",
            "a.example.com",
            "a&token=x",
            "a b",
            "é",
        ] {
            assert!(duckdns_label(bad).is_none(), "{bad}");
        }
        assert!(duckdns_label(&"a".repeat(64)).is_none());
    }
}
