//! Offline, targeted, signed enrollment. A code is not a bearer credential.
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use iroh::{EndpointId, SecretKey, Signature};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::Route;

pub const MAX_CODE_BYTES: usize = 2048;
pub const MAX_TTL_SECONDS: u64 = 900;
const MIN_TTL_SECONDS: u64 = 60;
const VERSION: u8 = 1;
const INVITE_PREFIX: &str = "IL1I.";
const ACCEPT_PREFIX: &str = "IL1A.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    version: u8,
    host: String,
    controller: String,
    route: Route,
    nonce: [u8; 16],
    created: u64,
    expires: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedCode {
    invitation: Invitation,
    signature: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CodeKind {
    Invite,
    Acceptance,
}

impl CodeKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Invite => INVITE_PREFIX,
            Self::Acceptance => ACCEPT_PREFIX,
        }
    }
}

impl Invitation {
    pub(crate) fn new(
        host: EndpointId,
        controller: EndpointId,
        route: Route,
        now: u64,
        ttl: u64,
    ) -> Result<Self> {
        ensure!(
            (MIN_TTL_SECONDS..=MAX_TTL_SECONDS).contains(&ttl),
            "TTL must be 60-900 seconds"
        );
        let mut nonce = [0; 16];
        let mut rng = OsRng;
        rng.try_fill_bytes(&mut nonce)
            .map_err(|_| anyhow::anyhow!("Operating system randomness unavailable"))?;
        let invite = Self {
            version: VERSION,
            host: host.to_string(),
            controller: controller.to_string(),
            route,
            nonce,
            created: now,
            expires: now.checked_add(ttl).context("Invalid invitation time")?,
        };
        invite.validate()?;
        Ok(invite)
    }

    pub fn host(&self) -> Result<EndpointId> {
        self.host.parse().context("Invalid host identity")
    }

    pub fn controller(&self) -> Result<EndpointId> {
        self.controller
            .parse()
            .context("Invalid controller identity")
    }

    pub fn route(&self) -> &Route {
        &self.route
    }
    pub fn expires(&self) -> u64 {
        self.expires
    }
    pub(crate) fn nonce(&self) -> [u8; 16] {
        self.nonce
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.version == VERSION, "Unknown invitation version");
        let host = self.host()?;
        let controller = self.controller()?;
        ensure!(host != controller, "Self-enrollment is forbidden");
        ensure!(
            host.to_string() == self.host && controller.to_string() == self.controller,
            "Noncanonical identity"
        );
        ensure!(
            Route::new(self.route.host(), self.route.port())? == self.route,
            "Noncanonical route"
        );
        let ttl = self
            .expires
            .checked_sub(self.created)
            .context("Invalid expiry")?;
        ensure!(
            (MIN_TTL_SECONDS..=MAX_TTL_SECONDS).contains(&ttl),
            "Invalid invitation lifetime"
        );
        Ok(())
    }

    pub fn check_time(&self, now: u64) -> Result<()> {
        ensure!(
            now >= self.created,
            "Invitation is not yet valid; check both device clocks"
        );
        ensure!(now < self.expires, "Invitation has expired");
        Ok(())
    }

    fn signer(&self, kind: CodeKind) -> Result<EndpointId> {
        match kind {
            CodeKind::Invite => self.host(),
            CodeKind::Acceptance => self.controller(),
        }
    }

    fn message(&self, kind: CodeKind) -> Result<Vec<u8>> {
        // A distinct domain separator prevents using a host offer as an acceptance.
        let mut bytes = format!("intralink/enrollment/v1/{}\0", kind.prefix()).into_bytes();
        bytes.extend(serde_json::to_vec(self)?);
        Ok(bytes)
    }

    pub(crate) fn encode(&self, kind: CodeKind, key: &SecretKey) -> Result<String> {
        self.validate()?;
        ensure!(key.public() == self.signer(kind)?, "Wrong signing identity");
        let signature = URL_SAFE_NO_PAD.encode(key.sign(&self.message(kind)?).to_bytes());
        let signed = SignedCode {
            invitation: self.clone(),
            signature,
        };
        let code = format!(
            "{}{}",
            kind.prefix(),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&signed)?)
        );
        ensure!(
            code.len() <= MAX_CODE_BYTES,
            "Invitation exceeds size limit"
        );
        Ok(code)
    }

    pub(crate) fn decode(code: &str, kind: CodeKind) -> Result<Self> {
        ensure!(
            code.len() <= MAX_CODE_BYTES,
            "Invitation exceeds size limit"
        );
        let payload = code
            .strip_prefix(kind.prefix())
            .context("Incorrect enrollment code type")?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .context("Invalid code encoding")?;
        let signed: SignedCode =
            serde_json::from_slice(&bytes).context("Invalid enrollment document")?;
        let invite = signed.invitation;
        invite.validate()?;
        let sig: [u8; 64] = URL_SAFE_NO_PAD
            .decode(signed.signature)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid signature length"))?;
        invite
            .signer(kind)?
            .verify(&invite.message(kind)?, &Signature::from_bytes(&sig))
            .context("Enrollment signature verification failed")?;
        Ok(invite)
    }

    /// Inspect a verified host offer before local approval; this alone grants no access.
    pub fn inspect_offer(code: &str, now: u64) -> Result<Self> {
        let invite = Self::decode(code, CodeKind::Invite)?;
        invite.check_time(now)?;
        Ok(invite)
    }

    /// Inspect a verified controller acceptance before local approval.
    pub fn inspect_acceptance(code: &str, now: u64) -> Result<Self> {
        let invite = Self::decode(code, CodeKind::Acceptance)?;
        invite.check_time(now)?;
        Ok(invite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_codes_reject_tampering_and_wrong_kind() -> Result<()> {
        let host = SecretKey::generate();
        let controller = SecretKey::generate();
        let invite = Invitation::new(
            host.public(),
            controller.public(),
            Route::new("localhost", 45100)?,
            1000,
            60,
        )?;
        let code = invite.encode(CodeKind::Invite, &host)?;
        assert_eq!(Invitation::inspect_offer(&code, 1059)?, invite);
        assert!(Invitation::inspect_offer(&code, 1060).is_err());
        assert!(Invitation::inspect_offer(&code, 999).is_err());
        assert!(Invitation::inspect_acceptance(&code, 1020).is_err());
        assert!(invite.encode(CodeKind::Invite, &controller).is_err());
        assert!(Invitation::inspect_offer(&"A".repeat(MAX_CODE_BYTES + 1), 1020).is_err());
        let mut document: SignedCode = serde_json::from_slice(
            &URL_SAFE_NO_PAD.decode(code.strip_prefix(INVITE_PREFIX).context("Missing prefix")?)?,
        )?;
        document.invitation.route = Route::new("different.example", 45100)?;
        let tampered = format!(
            "{INVITE_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&document)?)
        );
        assert!(Invitation::inspect_offer(&tampered, 1020).is_err());
        Ok(())
    }
}
