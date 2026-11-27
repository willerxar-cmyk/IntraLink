//! Copyable public connection information, never an authorization token.
//! Both owners still verify the full identity and separately approve pairing.
use std::net::{Ipv4Addr, SocketAddrV4};

pub const DEFAULT_PORT: u16 = 45100;
pub const MAX_CARD_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    identity: String,
    route: Option<SocketAddrV4>,
}

impl Card {
    pub fn new(identity: &str, route: Option<SocketAddrV4>) -> Option<Self> {
        if identity.len() != 64
            || !identity
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || route.is_some_and(|r| !r.ip().is_private() || r.port() == 0)
        {
            return None;
        }
        Some(Self {
            identity: identity.to_owned(),
            route,
        })
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub const fn route(&self) -> Option<SocketAddrV4> {
        self.route
    }

    pub fn encode(&self) -> String {
        match self.route {
            Some(route) => format!(
                "ILC1:CLIENT:{}:{}:{}",
                self.identity,
                route.ip(),
                route.port()
            ),
            None => format!("ILC1:BASE:{}", self.identity),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        if value.len() > MAX_CARD_BYTES || !value.is_ascii() {
            return None;
        }
        let mut parts = value.split(':');
        if parts.next()? != "ILC1" {
            return None;
        }
        let role = parts.next()?;
        let identity = parts.next()?;
        let route = match role {
            "BASE" => None,
            "CLIENT" => {
                let ip: Ipv4Addr = parts.next()?.parse().ok()?;
                let port: u16 = parts.next()?.parse().ok()?;
                Some(SocketAddrV4::new(ip, port))
            }
            _ => return None,
        };
        if parts.next().is_some() {
            return None;
        }
        let card = Self::new(identity, route)?;
        (card.encode() == value).then_some(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn exact_public_cards_round_trip_and_reject_noncanonical_input() {
        for route in [None, Some("192.168.7.2:45100".parse().unwrap())] {
            let card = Card::new(ID, route).unwrap();
            let text = card.encode();
            assert_eq!(Card::parse(&text), Some(card));
            assert!(Card::parse(&(text.clone() + ":extra")).is_none());
            assert!(Card::parse(&format!(" {text}")).is_none());
        }
        for route in ["8.8.8.8:45100", "127.0.0.1:45100", "10.0.0.1:0"] {
            assert!(Card::new(ID, Some(route.parse().unwrap())).is_none());
        }
        for suffix in [
            "192.168.7.2:045100",
            "192.168.007.2:45100",
            "192.168.7.2:65536",
        ] {
            assert!(Card::parse(&format!("ILC1:CLIENT:{ID}:{suffix}")).is_none());
        }
        assert!(Card::parse(&format!("ILC1:BASE:{}", ID.to_uppercase())).is_none());
        assert!(Card::parse(&format!("ILC1:HYBRID:{ID}")).is_none());
    }
}
