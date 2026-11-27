//! Explicitly selected private-LAN, diagnostic-only acceptance peer.
//! Fresh in-memory identity and local grant; no user vault, capture or input.
use anyhow::{Context, Result, ensure};
use intralink_core::Role;
use intralink_transport::{bind, bind_controller, receive_probe_with_wait, send_probe};
use intralink_vault::{Direction, Peer, Profile, Route};
use iroh::EndpointId;
use std::{
    io::{self, Write},
    net::SocketAddr,
    time::{Duration, Instant},
};

fn private_socket(value: &str) -> Result<SocketAddr> {
    let address: SocketAddr = value
        .parse()
        .context("Use a numeric private IPv4 address and port")?;
    ensure!(
        matches!(address, SocketAddr::V4(v) if v.ip().is_private()) && address.port() != 0,
        "Only explicit private IPv4 routes are accepted"
    );
    Ok(address)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 3 || (args.len() == 4 && matches!(args[3].as_str(), "media" | "surface")),
        "Usage: lan_probe_acceptance <host|base> <local IPv4:port> <remote IPv4:port> [media|surface]"
    );
    let media = args.len() == 4;
    let surface = args.get(3).is_some_and(|a| a == "surface");
    ensure!(
        !media || cfg!(feature = "lan-fixture"),
        "Media requires the explicit lan-fixture feature"
    );
    let local = private_socket(&args[1])?;
    let remote = private_socket(&args[2])?;
    ensure!(
        local.ip() != remote.ip(),
        "Acceptance requires distinct devices"
    );
    let host = match args[0].as_str() {
        "host" => true,
        "base" => false,
        _ => anyhow::bail!("Invalid role"),
    };
    ensure!(!surface || !host, "Surface fixture is Android source only");
    let mut profile = Profile::new(if host { Role::Host } else { Role::Controller });
    let endpoint = if host {
        #[cfg(feature = "lan-fixture")]
        if media {
            intralink_transport::desktop::bind_view_with_protocol(
                profile.secret_key(),
                local,
                intralink_transport::media_fixture::PROTOCOL,
            )
            .await?
        } else {
            bind(profile.secret_key(), local, true).await?
        }
        #[cfg(not(feature = "lan-fixture"))]
        bind(profile.secret_key(), local, true).await?
    } else {
        bind_controller(profile.secret_key(), &[remote]).await?
    };
    println!("LAN_WINDOWS_ID {}", endpoint.id());
    io::stdout().flush()?;
    // Only a public identity from the explicitly selected test device is
    // accepted. The coordinating process owns this stdin and its deadline.
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let peer: EndpointId = line
        .trim()
        .parse()
        .context("Expected Android public identity")?;
    ensure!(
        peer.to_string() == line.trim() && peer != endpoint.id(),
        "Invalid pinned identity"
    );
    let direction = if host {
        Direction::Inbound
    } else {
        Direction::Outbound
    };
    let route = if host {
        None
    } else {
        Some(Route::new(&remote.ip().to_string(), remote.port())?)
    };
    profile.add_peer(Peer::new("android-test", peer, direction, route)?)?;
    let grant = profile.peer("android-test", direction)?;
    let start = Instant::now();
    #[cfg(feature = "lan-fixture")]
    if media {
        let result = tokio::time::timeout(Duration::from_secs(35), async {
            if host {
                intralink_transport::media_fixture::serve(&endpoint, grant.identity()?).await
            } else if surface {
                intralink_transport::media_fixture::view_surface(
                    &endpoint,
                    grant.identity()?,
                    remote,
                )
                .await
            } else {
                intralink_transport::media_fixture::view(&endpoint, grant.identity()?, remote).await
            }
        })
        .await;
        endpoint.close().await;
        let stats = result??;
        println!(
            "LAN_WINDOWS_PASSED {} {}ms frames={} bytes={}",
            if host { "host" } else { "base" },
            start.elapsed().as_millis(),
            stats.frames,
            stats.media_bytes
        );
        return Ok(());
    }
    let result = if host {
        receive_probe_with_wait(&endpoint, grant.identity()?, Duration::from_secs(25)).await
    } else {
        send_probe(&endpoint, grant.identity()?, remote).await
    };
    endpoint.close().await;
    result?;
    println!(
        "LAN_WINDOWS_PASSED {} {}ms",
        if host { "host" } else { "base" },
        start.elapsed().as_millis()
    );
    Ok(())
}
