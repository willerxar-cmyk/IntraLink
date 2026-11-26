//! Version-specific handshakes. The caller has verified the pinned identity.
use super::{
    IO_LIMIT, REQUEST_LIMIT,
    profile::{RequestedMode, SessionProfile},
};
use anyhow::{Context, Result, ensure};
use intralink_core::{
    adaptive::ViewProtocol,
    capabilities::CodecCapabilities,
    desktop::{CONFIG_BYTES, REQUEST_BYTES, VideoConfig},
    h264,
    profile_limits::{self, ReceiverLimits},
};
use iroh::endpoint::{RecvStream, SendStream};
use tokio::time::timeout;

pub(super) async fn host(
    tx: &mut SendStream,
    rx: &mut RecvStream,
    config: SessionProfile,
) -> Result<SessionProfile> {
    let requested = config.requested();
    let mut request = [0; REQUEST_BYTES];
    timeout(REQUEST_LIMIT, rx.read_exact(&mut request)).await??;
    let config = match config {
        SessionProfile::Desktop(ceiling, ViewProtocol::Bounded) => {
            let offered = profile_limits::parse_request(&request)
                .context("Invalid bounded software codec offer")?;
            let mut bytes = [0; profile_limits::LIMIT_BYTES];
            timeout(REQUEST_LIMIT, rx.read_exact(&mut bytes)).await??;
            let limits =
                ReceiverLimits::decode(bytes).context("Invalid receiver resource limits")?;
            let chosen = limits
                .select(ceiling, offered)
                .context("No mutually approved software codec")?;
            let selection = profile_limits::selection(ceiling, chosen);
            timeout(IO_LIMIT, tx.write_all(&selection)).await??;
            let mut confirmation = [0; profile_limits::SELECTION_BYTES];
            timeout(REQUEST_LIMIT, rx.read_exact(&mut confirmation)).await??;
            ensure!(
                confirmation == selection,
                "Bounded profile confirmation mismatch"
            );
            SessionProfile::Desktop(chosen, ViewProtocol::Bounded)
        }
        SessionProfile::Desktop(ceiling, ViewProtocol::Negotiated) => {
            let remote = CodecCapabilities::parse_request(&request)
                .context("Invalid software codec capabilities")?;
            let local = CodecCapabilities::through(ceiling.mode());
            let selected = local
                .select(remote)
                .context("No mutually approved software codec")?;
            let config = ceiling.with_mode(selected);
            // The viewer verifies the intersection and echoes the exact
            // selection before any capture demand can be issued.
            let mut selection = [0; CONFIG_BYTES + 1];
            selection[0] = local.bits();
            selection[1..].copy_from_slice(&config.encode());
            timeout(IO_LIMIT, tx.write_all(&selection)).await??;
            let mut confirmation = [0; CONFIG_BYTES + 1];
            timeout(REQUEST_LIMIT, rx.read_exact(&mut confirmation)).await??;
            ensure!(
                confirmation == selection,
                "Codec selection confirmation mismatch"
            );
            SessionProfile::Desktop(config, ViewProtocol::Negotiated)
        }
        SessionProfile::Desktop(legacy, protocol) => {
            ensure!(
                request == requested.request(),
                "Unsupported or unapproved media mode"
            );
            timeout(IO_LIMIT, tx.write_all(&legacy.encode())).await??;
            SessionProfile::Desktop(legacy, protocol)
        }
        SessionProfile::H264(profile) => {
            ensure!(
                request == requested.request(),
                "Unsupported or unapproved media mode"
            );
            // The viewer explicitly requests the exact bounded profile,
            // before the Host is permitted to ask its local source.
            let mut offer = [0; h264::CONFIG_BYTES];
            timeout(REQUEST_LIMIT, rx.read_exact(&mut offer)).await??;
            ensure!(
                h264::Profile::decode(offer) == Some(profile),
                "H.264 profile offer not approved"
            );
            timeout(IO_LIMIT, tx.write_all(&profile.encode())).await??;
            SessionProfile::H264(profile)
        }
    };
    Ok(config)
}

pub(super) async fn viewer(
    tx: &mut SendStream,
    rx: &mut RecvStream,
    mode: RequestedMode,
) -> Result<SessionProfile> {
    timeout(REQUEST_LIMIT, tx.write_all(&mode.request())).await??;
    match mode {
        RequestedMode::H264(profile) => {
            timeout(REQUEST_LIMIT, tx.write_all(&profile.encode())).await??;
        }
        RequestedMode::Desktop(requested) if requested.protocol == ViewProtocol::Bounded => {
            timeout(REQUEST_LIMIT, tx.write_all(&requested.limits.encode())).await??;
        }
        _ => {}
    }
    let config = match mode {
        RequestedMode::Desktop(requested) if requested.protocol == ViewProtocol::Bounded => {
            let mut selection = [0; profile_limits::SELECTION_BYTES];
            timeout(IO_LIMIT, rx.read_exact(&mut selection)).await??;
            let offered = CodecCapabilities::through(requested.codec);
            let chosen = profile_limits::validate_selection(requested.limits, offered, selection)
                .context("Host selection violates the bounded profile intersection")?;
            timeout(IO_LIMIT, tx.write_all(&selection)).await??;
            SessionProfile::Desktop(chosen, ViewProtocol::Bounded)
        }
        RequestedMode::Desktop(requested) if requested.protocol == ViewProtocol::Negotiated => {
            let mut selection = [0; CONFIG_BYTES + 1];
            timeout(IO_LIMIT, rx.read_exact(&mut selection)).await??;
            let host = CodecCapabilities::from_bits(selection[0])
                .context("Invalid host software capabilities")?;
            let config = VideoConfig::decode(selection[1..].try_into()?)
                .context("Invalid negotiated configuration")?;
            let local = CodecCapabilities::through(requested.codec);
            ensure!(
                local.select(host) == Some(config.mode()),
                "Host selected a codec outside the deterministic intersection"
            );
            timeout(IO_LIMIT, tx.write_all(&selection)).await??;
            SessionProfile::Desktop(config, requested.protocol)
        }
        RequestedMode::Desktop(requested) => {
            let mut header = [0; CONFIG_BYTES];
            timeout(IO_LIMIT, rx.read_exact(&mut header)).await??;
            let config = VideoConfig::decode(header).context("Invalid desktop configuration")?;
            ensure!(
                config.mode() == requested.codec,
                "Host selected a different media mode"
            );
            SessionProfile::Desktop(config, requested.protocol)
        }
        RequestedMode::H264(expected) => {
            let mut header = [0; h264::CONFIG_BYTES];
            timeout(IO_LIMIT, rx.read_exact(&mut header)).await??;
            let profile = h264::Profile::decode(header).context("Invalid H.264 configuration")?;
            ensure!(
                profile == expected,
                "Host H.264 profile differs from approved diagnostic profile"
            );
            SessionProfile::H264(profile)
        }
    };
    Ok(config)
}
