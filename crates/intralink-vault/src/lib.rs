//! Local, password-encrypted identities and bilateral diagnostic grants.
#![forbid(unsafe_code)]

mod envelope;
mod invitation;
mod profile;
#[cfg(test)]
mod session_lease_tests;
mod store;

pub use invitation::{Invitation, MAX_CODE_BYTES, MAX_TTL_SECONDS};
pub use profile::{Direction, Peer, Profile, RoleChange, Route};
pub use store::{SessionKind, SessionVault, Vault};
