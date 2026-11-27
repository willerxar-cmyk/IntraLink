//! One-shot pinned-identity AVC capability agreement, loopback diagnostic only.
//! No frame messages, capture request, decoder callback, or persisted grant.
use crate::{HANDSHAKE_LIMIT, bind_for_protocol};
use anyhow::{Context, Result, bail, ensure};
use intralink_core::h264::{
    modes::Mode,
    negotiation::{self, ALPN, COMPLETE, OFFER_BYTES, Offer, RESPONSE_BYTES, Role},
};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use std::{net::SocketAddr, time::Duration};
use tokio::time::timeout;

const ACCEPT_LIMIT: Duration = Duration::from_secs(20);
const EXCHANGE_LIMIT: Duration = Duration::from_secs(5);

fn require_local(endpoint: &Endpoint) -> Result<()> {
    let sockets = endpoint.bound_sockets();
    ensure!(
        !sockets.is_empty() && sockets.iter().all(|s| s.ip().is_loopback()),
        "AVC negotiation diagnostic requires loopback-only sockets"
    );
    Ok(())
}

pub async fn bind(secret: SecretKey, addr: SocketAddr) -> Result<Endpoint> {
    ensure!(
        addr.ip().is_loopback(),
        "AVC negotiation binds loopback only"
    );
    let endpoint = bind_for_protocol(secret, addr, Some(ALPN)).await?;
    if let Err(error) = require_local(&endpoint) {
        endpoint.close().await;
        return Err(error);
    }
    Ok(endpoint)
}

/// The caller supplies an independently approved local offer and pinned peer
/// for this one exchange. The returned mode is only a negotiation result.
pub async fn receive(endpoint: &Endpoint, approved: EndpointId, offer: &Offer) -> Result<Mode> {
    require_local(endpoint)?;
    ensure!(
        offer.role() == Role::Producer,
        "Expected local producer offer"
    );
    let incoming = timeout(ACCEPT_LIMIT, endpoint.accept())
        .await?
        .context("Negotiation endpoint closed")?;
    let conn = timeout(HANDSHAKE_LIMIT, async { incoming.await }).await??;
    if conn.remote_id() != approved || conn.alpn() != ALPN {
        conn.close(1u32.into(), b"not authorized");
        bail!("Unapproved AVC negotiation peer or protocol");
    }
    let result = timeout(EXCHANGE_LIMIT, async {
        let (mut tx, mut rx) = conn.accept_bi().await?;
        let mut bytes = [0; OFFER_BYTES];
        rx.read_exact(&mut bytes).await?;
        let receiver = Offer::decode(&bytes).context("Invalid receiver offer")?;
        let response =
            negotiation::response(offer, &receiver).context("No common approved AVC mode")?;
        tx.write_all(&response).await?;
        // Exact confirmation and FIN. An appended byte cannot authorize even
        // this diagnostic agreement. No frame parser exists in this protocol.
        let confirmation = rx.read_to_end(RESPONSE_BYTES).await?;
        ensure!(
            confirmation.as_slice() == response,
            "AVC confirmation differs"
        );
        tx.write_all(COMPLETE).await?;
        tx.finish()?;
        let _ = conn.closed().await;
        negotiation::choose(offer, &receiver).context("No common AVC mode")
    })
    .await;
    if !matches!(&result, Ok(Ok(_))) {
        conn.close(2u32.into(), b"negotiation failed");
    }
    result.context("AVC negotiation timed out")?
}

pub async fn request(
    endpoint: &Endpoint,
    expected: EndpointId,
    remote: SocketAddr,
    offer: &Offer,
) -> Result<Mode> {
    require_local(endpoint)?;
    ensure!(
        remote.ip().is_loopback(),
        "AVC negotiation target must be loopback"
    );
    ensure!(
        offer.role() == Role::Receiver,
        "Expected local receiver offer"
    );
    let conn = timeout(
        HANDSHAKE_LIMIT,
        endpoint.connect(EndpointAddr::new(expected).with_ip_addr(remote), ALPN),
    )
    .await??;
    if conn.remote_id() != expected || conn.alpn() != ALPN {
        conn.close(1u32.into(), b"not authorized");
        bail!("Unexpected AVC negotiation identity or protocol");
    }
    let result = timeout(EXCHANGE_LIMIT, async {
        let (mut tx, mut rx) = conn.open_bi().await?;
        tx.write_all(&offer.encode()).await?;
        let mut response = [0; RESPONSE_BYTES];
        rx.read_exact(&mut response).await?;
        let selected = negotiation::confirm(offer, &response)
            .context("Invalid exact AVC mode intersection")?;
        tx.write_all(&response).await?;
        tx.finish()?;
        let complete = rx.read_to_end(COMPLETE.len()).await?;
        ensure!(complete == COMPLETE, "AVC negotiation not completed");
        Ok::<_, anyhow::Error>(selected)
    })
    .await;
    if matches!(&result, Ok(Ok(_))) {
        conn.close(0u32.into(), b"negotiation complete");
    } else {
        conn.close(2u32.into(), b"negotiation failed");
    }
    result.context("AVC negotiation timed out")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use intralink_core::h264::{
        SpsInfo,
        modes::{Color, Limits},
    };
    fn mode(w: u16, h: u16) -> Mode {
        Mode::new(
            SpsInfo {
                width: w,
                height: h,
                profile_idc: 66,
                level_idc: 41,
                constraints: 0,
                references: 2,
                poc_type: 2,
            },
            Color::Bt709Limited,
            30,
            8_000_000,
        )
        .unwrap()
    }
    fn offers() -> (Offer, Offer, Mode) {
        let a = mode(1920, 1080);
        let b = mode(1080, 1920);
        let limits = Limits::new(1920, 1920, 30, 12_000_000).unwrap();
        (
            Offer::new(Role::Producer, limits, &[a, b]).unwrap(),
            Offer::new(Role::Receiver, limits, &[b]).unwrap(),
            b,
        )
    }
    async fn endpoints() -> Result<(Endpoint, Endpoint, SocketAddr)> {
        let host = bind(SecretKey::generate(), "127.0.0.1:0".parse()?).await?;
        let viewer = crate::bind(SecretKey::generate(), "127.0.0.1:0".parse()?, false).await?;
        let addr = host
            .bound_sockets()
            .into_iter()
            .find(SocketAddr::is_ipv4)
            .context("Missing local socket")?;
        Ok((host, viewer, addr))
    }
    #[tokio::test]
    async fn pinned_peers_agree_on_exact_portrait_mode() -> Result<()> {
        let (host, viewer, addr) = endpoints().await?;
        let (p, r, expected) = offers();
        let (a, b) = tokio::join!(
            receive(&host, viewer.id(), &p),
            request(&viewer, host.id(), addr, &r)
        );
        assert_eq!(a?, expected);
        assert_eq!(b?, expected);
        viewer.close().await;
        host.close().await;
        Ok(())
    }
    #[tokio::test]
    async fn incorrect_identity_and_nonloopback_are_refused() -> Result<()> {
        assert!(
            bind(SecretKey::generate(), "0.0.0.0:0".parse()?)
                .await
                .is_err()
        );
        let (host, viewer, addr) = endpoints().await?;
        let (p, r, _) = offers();
        assert!(
            request(&viewer, host.id(), "192.0.2.1:1234".parse()?, &r)
                .await
                .is_err()
        );
        let (a, b) = tokio::join!(
            receive(&host, SecretKey::generate().public(), &p),
            request(&viewer, host.id(), addr, &r)
        );
        assert!(a.is_err() && b.is_err());
        viewer.close().await;
        host.close().await;
        Ok(())
    }
    #[tokio::test]
    async fn mismatched_or_trailing_confirmation_never_completes() -> Result<()> {
        for trailing in [false, true] {
            let (host, viewer, addr) = endpoints().await?;
            let (p, r, _) = offers();
            let attack = async {
                let c = viewer
                    .connect(EndpointAddr::new(host.id()).with_ip_addr(addr), ALPN)
                    .await?;
                let (mut tx, mut rx) = c.open_bi().await?;
                tx.write_all(&r.encode()).await?;
                let mut response = [0; RESPONSE_BYTES];
                rx.read_exact(&mut response).await?;
                if !trailing {
                    response[RESPONSE_BYTES - 1] ^= 1;
                }
                tx.write_all(&response).await?;
                if trailing {
                    tx.write_all(&[0]).await?;
                }
                tx.finish()?;
                assert!(rx.read_to_end(COMPLETE.len()).await.is_err());
                Ok::<_, anyhow::Error>(())
            };
            let (a, b) = tokio::join!(receive(&host, viewer.id(), &p), attack);
            assert!(a.is_err());
            b?;
            viewer.close().await;
            host.close().await;
        }
        Ok(())
    }
    #[tokio::test]
    async fn no_common_mode_refuses_agreement() -> Result<()> {
        let (host, viewer, addr) = endpoints().await?;
        let (p, _, _) = offers();
        let r = Offer::new(
            Role::Receiver,
            Limits::new(1280, 720, 30, 12_000_000).unwrap(),
            &[mode(1280, 720)],
        )
        .unwrap();
        let (a, b) = tokio::join!(
            receive(&host, viewer.id(), &p),
            request(&viewer, host.id(), addr, &r)
        );
        assert!(a.is_err() && b.is_err());
        viewer.close().await;
        host.close().await;
        Ok(())
    }
}
