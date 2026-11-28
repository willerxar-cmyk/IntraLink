//! Offline enrollment shared by Android JNI and host tests.
#![forbid(unsafe_code)]
use anyhow::{Context, Result, ensure};
use intralink_vault::{Direction, Invitation, MAX_CODE_BYTES, Peer, Profile, Route, Vault};
use iroh::EndpointId;
use qrcode::{Color, QrCode};
use serde::Deserialize;
use serde_json::{Value, json};
#[cfg(target_os = "android")]
use std::time::{SystemTime, UNIX_EPOCH};
use std::{net::Ipv4Addr, path::Path};

const TTL: u64 = 300;
const MAX_REQUEST: usize = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, tag = "stage", rename_all = "snake_case")]
pub(crate) enum Request {
    Issue {
        local_id: String,
        controller_id: String,
        host: String,
        port: u16,
    },
    Accept {
        local_id: String,
        name: String,
        code: String,
    },
    Confirm {
        local_id: String,
        name: String,
        code: String,
    },
    Cancel {
        local_id: String,
    },
}

pub(crate) fn decode(value: &str) -> Result<Request> {
    ensure!(
        value.len() <= MAX_REQUEST,
        "Enrollment request exceeds limit"
    );
    serde_json::from_str(value).context("Invalid enrollment request")
}

#[cfg(target_os = "android")]
pub(crate) fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Invalid device clock")?
        .as_secs())
}

fn id(value: &str) -> Result<EndpointId> {
    let parsed: EndpointId = value.parse().context("Invalid public identity")?;
    ensure!(
        parsed.to_string() == value,
        "Use a complete canonical public identity"
    );
    Ok(parsed)
}

fn route(host: &str, port: u16) -> Result<Route> {
    let ip: Ipv4Addr = host.parse().context("Use numeric private IPv4")?;
    ensure!(
        ip.is_private() && ip.to_string() == host && port != 0,
        "Private IPv4 and nonzero port required"
    );
    Route::new(host, port)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Issue,
    Accept,
    Confirm,
    Cancel,
}

struct Review {
    stage: Stage,
    local: EndpointId,
    host: Option<EndpointId>,
    controller: Option<EndpointId>,
    route: Option<Route>,
    name: Option<String>,
    expires: Option<u64>,
}

impl Review {
    fn value(&self) -> Value {
        json!({
            "ok": true,
            "stage": match self.stage { Stage::Issue=>"issue",Stage::Accept=>"accept",Stage::Confirm=>"confirm",Stage::Cancel=>"cancel" },
            "local_id": self.local.to_string(),
            "host_id": self.host.map(|id|id.to_string()),
            "controller_id": self.controller.map(|id|id.to_string()),
            "route": self.route,
            "name": self.name,
            "expires": self.expires,
            "ttl": if self.stage==Stage::Issue { Some(TTL) } else { None },
        })
    }
}

fn review(request: &Request, at: u64) -> Result<Review> {
    match request {
        Request::Issue {
            local_id,
            controller_id,
            host,
            port,
        } => {
            let local = id(local_id)?;
            let controller = id(controller_id)?;
            ensure!(local != controller, "Self-enrollment is forbidden");
            Ok(Review {
                stage: Stage::Issue,
                local,
                host: Some(local),
                controller: Some(controller),
                route: Some(route(host, *port)?),
                name: None,
                expires: None,
            })
        }
        Request::Accept {
            local_id,
            name,
            code,
        }
        | Request::Confirm {
            local_id,
            name,
            code,
        } => {
            let local = id(local_id)?;
            ensure!(
                !code.is_empty() && code.len() <= MAX_CODE_BYTES,
                "Invalid signed code length"
            );
            let accept = matches!(request, Request::Accept { .. });
            let invitation = if accept {
                Invitation::inspect_offer(code, at)?
            } else {
                Invitation::inspect_acceptance(code, at)?
            };
            let host = invitation.host()?;
            let controller = invitation.controller()?;
            ensure!(
                local == if accept { controller } else { host },
                "Code targets a different device"
            );
            let route = route(invitation.route().host(), invitation.route().port())?;
            if accept {
                Peer::new(name, host, Direction::Outbound, Some(route.clone()))?;
            } else {
                Peer::new(name, controller, Direction::Inbound, None)?;
            }
            Ok(Review {
                stage: if accept {
                    Stage::Accept
                } else {
                    Stage::Confirm
                },
                local,
                host: Some(host),
                controller: Some(controller),
                route: Some(route),
                name: Some(name.clone()),
                expires: Some(invitation.expires()),
            })
        }
        Request::Cancel { local_id } => Ok(Review {
            stage: Stage::Cancel,
            local: id(local_id)?,
            host: None,
            controller: None,
            route: None,
            name: None,
            expires: None,
        }),
    }
}

/// Read-only verification. Does not unlock a vault, create a grant, or use the network.
pub(crate) fn inspect(request: &Request, at: u64) -> Result<Value> {
    Ok(review(request, at)?.value())
}

fn code_value(code: &str, expires: u64) -> Result<Value> {
    ensure!(code.len() <= MAX_CODE_BYTES, "Invalid QR payload");
    let qr = QrCode::new(code.as_bytes())?;
    let width = qr.width();
    ensure!((21..=177).contains(&width), "Invalid QR dimensions");
    let mut modules = String::with_capacity(width * width);
    for y in 0..width {
        for x in 0..width {
            modules.push(if qr[(x, y)] == Color::Dark { '1' } else { '0' });
        }
    }
    Ok(json!({"code":code,"qr_width":width,"qr_modules":modules,"expires":expires}))
}

/// Commit only the exact immutable request reviewed by the foreground UI.
/// Rechecks the canonical local identity, route, signature, roles, expiry
/// and one-time pending/replay state inside the existing locked vault.
pub(crate) fn commit(
    path: &Path,
    password: &[u8],
    request: Request,
    at: u64,
    mut clock: impl FnMut() -> Result<u64>,
) -> Result<(Profile, Value)> {
    let inspected = review(&request, at)?;
    let vault = Vault::lock(path)?;
    let mut profile = vault.load(password)?;
    ensure!(
        profile.public_id() == inspected.local,
        "Local identity changed; review again"
    );
    // Slow password derivation must not allow an expired invitation.
    let at = clock()?;
    let inspected = review(&request, at)?;
    let value = match request {
        Request::Issue { .. } => {
            let code = profile.create_invite(
                inspected.controller.context("Missing controller")?,
                inspected.route.context("Missing route")?,
                at,
                TTL,
            )?;
            code_value(&code, Invitation::inspect_offer(&code, at)?.expires())?
        }
        Request::Accept { name, code, .. } => {
            let acceptance = profile.accept_invite(&name, &code, at)?;
            code_value(&acceptance, inspected.expires.context("Missing expiry")?)?
        }
        Request::Confirm { name, code, .. } => {
            profile.confirm_invite(&name, &code, at)?;
            json!({})
        }
        Request::Cancel { .. } => {
            ensure!(profile.role().can_receive(), "CLIENT role required");
            profile.cancel_invite()?;
            json!({})
        }
    };
    if let Some(expires) = inspected.expires {
        ensure!(clock()? < expires, "Invitation expired before saving");
    }
    vault.save(&profile, password)?;
    Ok((profile, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use intralink_core::Role;

    #[test]
    fn full_exchange_review_qr_and_replay_with_exact_persistent_vaults() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let client_path = directory.path().join("client");
        let base_path = directory.path().join("base");
        let client = Profile::new(Role::Host);
        let base = Profile::new(Role::Controller);
        let password = b"disposable-test-passphrase";
        Vault::lock(&client_path)?.initialize(&client, password)?;
        Vault::lock(&base_path)?.initialize(&base, password)?;
        let issue = Request::Issue {
            local_id: client.public_id().to_string(),
            controller_id: base.public_id().to_string(),
            host: "192.168.2.3".to_owned(),
            port: 45100,
        };
        let before = std::fs::read(&client_path)?;
        let preview = inspect(&issue, 1000)?;
        assert_eq!(preview["controller_id"], base.public_id().to_string());
        assert_eq!(before, std::fs::read(&client_path)?);
        let (_, issued) = commit(&client_path, password, issue, 1000, || Ok(1000))?;
        let code = issued["code"].as_str().context("offer")?.to_owned();
        let width = issued["qr_width"].as_u64().context("width")? as usize;
        let modules = issued["qr_modules"].as_str().context("modules")?;
        let qr = QrCode::new(code.as_bytes())?;
        assert_eq!(width, qr.width());
        assert_eq!(modules.len(), width * width);
        for y in 0..width {
            for x in 0..width {
                assert_eq!(
                    modules.as_bytes()[y * width + x],
                    if qr[(x, y)] == Color::Dark {
                        b'1'
                    } else {
                        b'0'
                    }
                );
            }
        }
        let accept = || Request::Accept {
            local_id: base.public_id().to_string(),
            name: "client".into(),
            code: code.clone(),
        };
        let base_before = std::fs::read(&base_path)?;
        assert!(inspect(&accept(), 1300).is_err());
        assert!(commit(&base_path, password, accept(), 1001, || Ok(1300)).is_err());
        assert!(commit(&base_path, b"wrong-password", accept(), 1001, || Ok(1001)).is_err());
        assert_eq!(base_before, std::fs::read(&base_path)?);
        let (profile, accepted) = commit(&base_path, password, accept(), 1001, || Ok(1001))?;
        assert_eq!(profile.peers().len(), 1);
        assert!(
            Vault::lock(&client_path)?
                .load(password)?
                .peers()
                .is_empty()
        );
        let reply = accepted["code"].as_str().context("acceptance")?.to_owned();
        let confirm = || Request::Confirm {
            local_id: client.public_id().to_string(),
            name: "base".into(),
            code: reply.clone(),
        };
        assert!(
            inspect(
                &Request::Accept {
                    local_id: client.public_id().to_string(),
                    name: "wrong".into(),
                    code: code.clone()
                },
                1001
            )
            .is_err()
        );
        assert!(
            inspect(
                &Request::Confirm {
                    local_id: client.public_id().to_string(),
                    name: "wrong".into(),
                    code: code.clone()
                },
                1001
            )
            .is_err()
        );
        let (profile, _) = commit(&client_path, password, confirm(), 1002, || Ok(1002))?;
        assert_eq!(profile.peers().len(), 1);
        assert!(commit(&client_path, password, confirm(), 1003, || Ok(1003)).is_err());
        assert!(commit(&base_path, password, accept(), 1003, || Ok(1003)).is_err());
        Ok(())
    }

    #[test]
    fn route_identity_and_code_bounds() -> Result<()> {
        let host = Profile::new(Role::Host);
        let base = Profile::new(Role::Controller);
        for address in ["127.0.0.1", "8.8.8.8", "192.168.01.2", "example.org"] {
            assert!(
                inspect(
                    &Request::Issue {
                        local_id: host.public_id().to_string(),
                        controller_id: base.public_id().to_string(),
                        host: address.into(),
                        port: 45100
                    },
                    1
                )
                .is_err()
            );
        }
        assert!(decode(&"x".repeat(MAX_REQUEST + 1)).is_err());
        assert!(
            inspect(
                &Request::Accept {
                    local_id: base.public_id().to_string(),
                    name: "client".into(),
                    code: "x".repeat(MAX_CODE_BYTES + 1)
                },
                1
            )
            .is_err()
        );
        assert!(
            inspect(
                &Request::Issue {
                    local_id: host.public_id().to_string(),
                    controller_id: host.public_id().to_string(),
                    host: "192.168.1.2".into(),
                    port: 45100
                },
                1
            )
            .is_err()
        );
        Ok(())
    }
}
