use std::net::IpAddr;

use anyhow::{Context, Result, ensure};
use intralink_core::Role;
use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::invitation::{CodeKind, Invitation};

const SCHEMA_VERSION: u16 = 1;
const MAX_PEERS: usize = 128;
const MAX_USED_INVITES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Inbound,
    Outbound,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    host: String,
    port: u16,
}

impl Route {
    pub fn new(host: &str, port: u16) -> Result<Self> {
        let host = host.to_ascii_lowercase();
        ensure!(port != 0, "Remote port must be nonzero");
        ensure!(valid_host(&host), "Invalid hostname or IP address");
        Ok(Self { host, port })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

fn valid_host(host: &str) -> bool {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return !ip.is_unspecified() && !ip.is_multicast();
    }
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    name: String,
    identity: String,
    direction: Direction,
    route: Option<Route>,
}

impl Peer {
    pub fn new(
        name: &str,
        identity: EndpointId,
        direction: Direction,
        route: Option<Route>,
    ) -> Result<Self> {
        let peer = Self {
            name: name.to_owned(),
            identity: identity.to_string(),
            direction,
            route,
        };
        peer.validate()?;
        Ok(peer)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn identity(&self) -> Result<EndpointId> {
        self.identity.parse().context("Invalid peer identity")
    }

    pub fn direction(&self) -> Direction {
        self.direction
    }

    pub fn route(&self) -> Option<&Route> {
        self.route.as_ref()
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.is_empty()
                && self.name.len() <= 48
                && self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "Peer name must be 1-48 ASCII letters, digits, underscores or hyphens"
        );
        let id = self.identity()?;
        ensure!(
            self.identity == id.to_string(),
            "Noncanonical peer identity"
        );
        match (&self.direction, &self.route) {
            (Direction::Inbound, None) => Ok(()),
            (Direction::Outbound, Some(route)) => {
                let normalized = Route::new(route.host(), route.port())?;
                ensure!(normalized == *route, "Noncanonical route");
                Ok(())
            }
            _ => anyhow::bail!("Only outbound grants may contain a route"),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UsedInvite {
    nonce: [u8; 16],
    expires: u64,
}

/// Summary of an explicit, local role transition. No new grants are created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleChange {
    pub previous: Role,
    pub next: Role,
    pub revoked_grants: usize,
    pub canceled_invite: bool,
}

/// All private material remains local; only `public_id` is intended for exchange.
/// Do not derive Debug or Clone for the profile.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    schema: u16,
    role: Role,
    private_key: [u8; 32],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_name: Option<String>,
    peers: Vec<Peer>,
    // Additive fields allow loading an original v0.2 vault without resetting identity.
    #[serde(default)]
    pending_invite: Option<Invitation>,
    #[serde(default)]
    used_invites: Vec<UsedInvite>,
}

impl Drop for Profile {
    fn drop(&mut self) {
        self.private_key.zeroize();
    }
}

impl Profile {
    pub fn new(role: Role) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            role,
            private_key: SecretKey::generate().to_bytes(),
            device_name: None,
            peers: Vec::new(),
            pending_invite: None,
            used_invites: Vec::new(),
        }
    }

    /// Create a named local identity. Names are metadata, never credentials.
    pub fn new_named(role: Role, name: &str) -> Result<Self> {
        let mut profile = Self::new(role);
        profile.rename_device(name)?;
        Ok(profile)
    }

    pub fn device_name(&self) -> Option<&str> {
        self.device_name.as_deref()
    }

    pub(crate) fn rename_device(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        validate_device_name(name)?;
        self.device_name = Some(name.to_owned());
        Ok(())
    }

    pub fn role_change_impact(&self, next: Role) -> RoleChange {
        RoleChange {
            previous: self.role,
            next,
            revoked_grants: self
                .peers
                .iter()
                .filter(|peer| !direction_allowed(next, peer.direction))
                .count(),
            canceled_invite: self.role != next && self.pending_invite.is_some(),
        }
    }

    /// Only the authenticated Vault migration path may modify role.
    /// Retain replay receipts even when a controller becomes a client.
    pub(crate) fn apply_role_change(&mut self, next: Role) -> RoleChange {
        let impact = self.role_change_impact(next);
        if self.role != next {
            self.peers
                .retain(|peer| direction_allowed(next, peer.direction));
            self.pending_invite = None;
            self.role = next;
        }
        impact
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn secret_key(&self) -> SecretKey {
        SecretKey::from_bytes(&self.private_key)
    }

    pub fn public_id(&self) -> EndpointId {
        self.secret_key().public()
    }

    pub fn peers(&self) -> &[Peer] {
        &self.peers
    }

    pub fn peer(&self, name: &str, direction: Direction) -> Result<&Peer> {
        self.peers
            .iter()
            .find(|peer| peer.name == name && peer.direction == direction)
            .context("No matching locally approved peer")
    }

    pub fn add_peer(&mut self, peer: Peer) -> Result<()> {
        self.validate_peer(&peer)?;
        ensure!(self.peers.len() < MAX_PEERS, "Peer limit reached");
        ensure!(
            !self.peers.iter().any(|existing| existing.name == peer.name),
            "Name already exists; revoke the previous grant before replacing it"
        );
        ensure!(
            !self.peers.iter().any(|existing| {
                existing.identity == peer.identity && existing.direction == peer.direction
            }),
            "Identity already has a grant in this direction"
        );
        self.peers.push(peer);
        Ok(())
    }

    pub fn revoke(&mut self, name: &str) -> Result<()> {
        let index = self
            .peers
            .iter()
            .position(|peer| peer.name == name)
            .context("No matching grant to revoke")?;
        self.peers.remove(index);
        Ok(())
    }

    /// Create one pending host offer. Replacing it invalidates the previous code.
    pub fn create_invite(
        &mut self,
        controller: EndpointId,
        route: Route,
        now: u64,
        ttl: u64,
    ) -> Result<String> {
        ensure!(
            self.role.can_receive(),
            "Only host or hybrid profiles can issue offers"
        );
        ensure!(
            !self
                .peers
                .iter()
                .any(|peer| peer.direction == Direction::Inbound
                    && peer.identity == controller.to_string()),
            "Controller already has an inbound grant"
        );
        let invite = Invitation::new(self.public_id(), controller, route, now, ttl)?;
        let code = invite.encode(CodeKind::Invite, &self.secret_key())?;
        self.pending_invite = Some(invite);
        Ok(code)
    }

    pub fn accept_invite(&mut self, name: &str, code: &str, now: u64) -> Result<String> {
        ensure!(
            self.role.can_initiate(),
            "Only controller or hybrid profiles can accept offers"
        );
        let invite = Invitation::inspect_offer(code, now)?;
        ensure!(
            invite.controller()? == self.public_id(),
            "Offer targets a different controller"
        );
        self.used_invites.retain(|used| used.expires > now);
        ensure!(
            !self
                .used_invites
                .iter()
                .any(|used| used.nonce == invite.nonce()),
            "Offer already accepted"
        );
        ensure!(
            self.used_invites.len() < MAX_USED_INVITES,
            "Recent invitation limit reached; wait for expiry"
        );
        let peer = Peer::new(
            name,
            invite.host()?,
            Direction::Outbound,
            Some(invite.route().clone()),
        )?;
        let acceptance = invite.encode(CodeKind::Acceptance, &self.secret_key())?;
        self.add_peer(peer)?;
        self.used_invites.push(UsedInvite {
            nonce: invite.nonce(),
            expires: invite.expires(),
        });
        Ok(acceptance)
    }

    /// Final host approval and consumption are persisted in the same vault write.
    pub fn confirm_invite(&mut self, name: &str, code: &str, now: u64) -> Result<()> {
        ensure!(
            self.role.can_receive(),
            "This profile cannot receive grants"
        );
        let invite = Invitation::inspect_acceptance(code, now)?;
        ensure!(
            invite.host()? == self.public_id(),
            "Acceptance targets a different host"
        );
        ensure!(
            self.pending_invite.as_ref() == Some(&invite),
            "No matching outstanding invitation (used, replaced or canceled)"
        );
        self.add_peer(Peer::new(
            name,
            invite.controller()?,
            Direction::Inbound,
            None,
        )?)?;
        self.pending_invite = None;
        Ok(())
    }

    pub fn cancel_invite(&mut self) -> Result<()> {
        ensure!(
            self.pending_invite.take().is_some(),
            "No outstanding invitation"
        );
        Ok(())
    }

    fn validate_peer(&self, peer: &Peer) -> Result<()> {
        peer.validate()?;
        ensure!(
            peer.identity()? != self.public_id(),
            "Self-pairing is forbidden"
        );
        ensure!(
            direction_allowed(self.role, peer.direction),
            "Grant is forbidden for this profile role"
        );
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.schema == SCHEMA_VERSION, "Unsupported profile schema");
        ensure!(self.peers.len() <= MAX_PEERS, "Too many peer grants");
        if let Some(invite) = &self.pending_invite {
            invite.validate()?;
            ensure!(
                self.role.can_receive() && invite.host()? == self.public_id(),
                "Invalid pending offer owner"
            );
        }
        ensure!(
            self.used_invites.len() <= MAX_USED_INVITES,
            "Too many invitation receipts"
        );
        // Historical receipts are inert in host-only mode. Never remove them
        // during role changes: returning to controller must not permit replay.
        if let Some(name) = &self.device_name {
            validate_device_name(name)?;
        }
        for (index, used) in self.used_invites.iter().enumerate() {
            ensure!(
                !self.used_invites[..index]
                    .iter()
                    .any(|previous| previous.nonce == used.nonce),
                "Duplicate invitation receipt"
            );
        }
        for (index, peer) in self.peers.iter().enumerate() {
            self.validate_peer(peer)?;
            ensure!(
                !self.peers[..index].iter().any(|previous| {
                    previous.name == peer.name
                        || (previous.identity == peer.identity
                            && previous.direction == peer.direction)
                }),
                "Duplicate peer grant"
            );
        }
        Ok(())
    }
}

fn direction_allowed(role: Role, direction: Direction) -> bool {
    match direction {
        Direction::Inbound => role.can_receive(),
        Direction::Outbound => role.can_initiate(),
    }
}

fn validate_device_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name == name.trim()
            && !name.chars().any(|c| c.is_control()
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')),
        "Device name must contain 1-64 UTF-8 bytes without control characters"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invitation_needs_both_approvals_and_cannot_be_reused() -> Result<()> {
        let mut host = Profile::new(Role::Host);
        let mut controller = Profile::new(Role::Controller);
        let mut stranger = Profile::new(Role::Controller);
        let offer = host.create_invite(
            controller.public_id(),
            Route::new("localhost", 45100)?,
            1000,
            300,
        )?;
        assert!(host.peers().is_empty());
        assert!(stranger.accept_invite("host", &offer, 1020).is_err());
        assert!(controller.accept_invite("host", &offer, 1300).is_err());
        let acceptance = controller.accept_invite("host", &offer, 1020)?;
        assert!(host.peers().is_empty());
        assert_eq!(
            controller.peer("host", Direction::Outbound)?.identity()?,
            host.public_id()
        );
        host.confirm_invite("owner", &acceptance, 1030)?;
        assert_eq!(
            host.peer("owner", Direction::Inbound)?.identity()?,
            controller.public_id()
        );
        assert!(host.peer("owner", Direction::Outbound).is_err());
        host.revoke("owner")?;
        controller.revoke("host")?;
        assert!(host.confirm_invite("owner", &acceptance, 1040).is_err());
        assert!(controller.accept_invite("host", &offer, 1040).is_err());
        Ok(())
    }

    #[test]
    fn canceled_or_replaced_offers_cannot_activate() -> Result<()> {
        let mut host = Profile::new(Role::Host);
        let mut controller = Profile::new(Role::Controller);
        let route = Route::new("localhost", 45100)?;
        let offer = host.create_invite(controller.public_id(), route.clone(), 1000, 300)?;
        let receipt = controller.accept_invite("host", &offer, 1001)?;
        let replacement = host.create_invite(controller.public_id(), route.clone(), 1002, 300)?;
        assert!(host.confirm_invite("owner", &receipt, 1003).is_err());
        controller.revoke("host")?;
        let receipt = controller.accept_invite("host", &replacement, 1004)?;
        host.cancel_invite()?;
        assert!(host.confirm_invite("owner", &receipt, 1005).is_err());
        assert!(host.peers().is_empty());
        let owner_id = host.public_id();
        assert!(
            controller
                .create_invite(owner_id, route, 1000, 300)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn legacy_profile_fields_are_accepted() -> Result<()> {
        let profile = Profile::new(Role::Host);
        let id = profile.public_id();
        // Reproduce v0.2's JSON field set; encryption envelope is unchanged.
        let mut old = serde_json::to_value(&profile)?;
        let object = old.as_object_mut().context("Expected JSON object")?;
        object.remove("pending_invite");
        object.remove("used_invites");
        let restored: Profile = serde_json::from_value(old)?;
        restored.validate()?;
        assert_eq!(restored.public_id(), id);
        assert!(restored.pending_invite.is_none());
        Ok(())
    }

    #[test]
    fn role_isolation_and_revocation() -> Result<()> {
        let mut host = Profile::new(Role::Host);
        let controller = Profile::new(Role::Controller);
        let route = Route::new("127.0.0.1", 45100)?;
        let outbound = Peer::new(
            "out",
            controller.public_id(),
            Direction::Outbound,
            Some(route),
        )?;
        assert!(host.add_peer(outbound).is_err());
        let inbound = Peer::new("owner", controller.public_id(), Direction::Inbound, None)?;
        host.add_peer(inbound.clone())?;
        assert!(host.add_peer(inbound).is_err());
        assert!(host.peer("owner", Direction::Outbound).is_err());
        assert_eq!(
            host.peer("owner", Direction::Inbound)?.identity()?,
            controller.public_id()
        );
        host.revoke("owner")?;
        assert!(host.peer("owner", Direction::Inbound).is_err());
        assert!(host.revoke("owner").is_err());
        Ok(())
    }

    #[test]
    fn rejects_invalid_routes_and_self_pairing() -> Result<()> {
        for invalid in [
            "",
            "https://host",
            "host:45100",
            "0.0.0.0",
            "::",
            "ff02::1",
            "a..b",
            "a b",
        ] {
            assert!(Route::new(invalid, 45100).is_err(), "{invalid}");
        }
        assert!(Route::new("example.net", 0).is_err());
        assert!(Route::new("2001:db8::1", 45100).is_ok());
        let mut hybrid = Profile::new(Role::Hybrid);
        let own_id = hybrid.public_id();
        assert!(
            hybrid
                .add_peer(Peer::new("self", own_id, Direction::Inbound, None)?)
                .is_err()
        );
        assert!(
            Peer::new(
                "bad",
                own_id,
                Direction::Inbound,
                Some(Route::new("localhost", 45100)?)
            )
            .is_err()
        );
        assert!(Peer::new("bad", own_id, Direction::Outbound, None).is_err());
        Ok(())
    }
}
