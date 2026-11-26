//! Session profile and record validation shared by sender and receiver.
use anyhow::{Result, bail, ensure};
use intralink_core::{
    adaptive::{self, ViewProtocol},
    desktop::{CodecMode, REQUEST_BYTES, VideoConfig},
    h264,
    pacing::CapturePacer,
};

/// An explicit wire version and codec; neither can change during a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewMode {
    pub codec: CodecMode,
    pub protocol: ViewProtocol,
}

impl From<CodecMode> for ViewMode {
    fn from(codec: CodecMode) -> Self {
        Self {
            codec,
            protocol: ViewProtocol::Fixed,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum SessionProfile {
    Desktop(VideoConfig, ViewProtocol),
    H264(h264::Profile),
}

#[derive(Clone, Copy)]
pub(super) enum RequestedMode {
    Desktop(ViewMode),
    H264(h264::Profile),
}

impl RequestedMode {
    pub(super) fn alpn(self) -> &'static [u8] {
        match self {
            Self::Desktop(mode) => mode.protocol.alpn(),
            Self::H264(_) => h264::ALPN,
        }
    }

    pub(super) fn request(self) -> [u8; REQUEST_BYTES] {
        match self {
            Self::Desktop(mode) => mode.protocol.request(mode.codec),
            Self::H264(_) => *h264::REQUEST,
        }
    }
}

impl SessionProfile {
    pub(super) fn requested(self) -> RequestedMode {
        match self {
            Self::Desktop(config, protocol) => RequestedMode::Desktop(ViewMode {
                codec: config.mode(),
                protocol,
            }),
            Self::H264(profile) => RequestedMode::H264(profile),
        }
    }

    pub(super) fn seconds(self) -> u64 {
        match self {
            Self::Desktop(config, _) => config.seconds(),
            Self::H264(profile) => profile.seconds(),
        }
    }

    pub(super) fn max_frames(self) -> usize {
        match self {
            Self::Desktop(config, _) => config.max_frames(),
            Self::H264(profile) => profile.max_frames(),
        }
    }

    pub(super) fn pacer(self) -> CapturePacer {
        match self {
            Self::Desktop(config, _) => CapturePacer::new(config),
            Self::H264(profile) => CapturePacer::for_h264(profile),
        }
    }

    pub(super) fn is_adaptive(self) -> bool {
        matches!(self, Self::Desktop(_, protocol) if protocol.is_adaptive())
    }

    pub(super) fn desktop_config(self) -> Option<VideoConfig> {
        match self {
            Self::Desktop(config, _) => Some(config),
            Self::H264(_) => None,
        }
    }

    pub(super) fn media_guard(self) -> MediaGuard {
        match self {
            Self::Desktop(config, ViewProtocol::Fixed) => MediaGuard::Fixed(config),
            Self::Desktop(config, protocol) => {
                MediaGuard::Adaptive(adaptive::StreamState::new(config), protocol)
            }
            Self::H264(profile) => MediaGuard::H264 {
                profile,
                state: Box::new(h264::StreamState::new(profile)),
            },
        }
    }
}

pub(super) struct MediaFrame {
    pub(super) profile: SessionProfile,
    pub(super) payload: Vec<u8>,
    pub(super) sequence: usize,
}

pub(super) enum MediaGuard {
    Fixed(VideoConfig),
    Adaptive(adaptive::StreamState, ViewProtocol),
    H264 {
        profile: h264::Profile,
        state: Box<h264::StreamState>,
    },
}

impl MediaGuard {
    pub(super) fn accept(&mut self, height: Option<u16>, bytes: &[u8]) -> Result<SessionProfile> {
        match self {
            Self::Fixed(config) => {
                ensure!(height.is_none(), "Adaptive frame not permitted in v8");
                Ok(SessionProfile::Desktop(*config, ViewProtocol::Fixed))
            }
            Self::Adaptive(state, protocol) => {
                let Some(height) = height else {
                    bail!("Missing v10 adaptive frame geometry");
                };
                let config = state.accept(height, bytes)?;
                Ok(SessionProfile::Desktop(config, *protocol))
            }
            Self::H264 { profile, state } => {
                ensure!(
                    height.is_none(),
                    "Adaptive frame not permitted in H.264 diagnostic"
                );
                state
                    .accept(bytes)
                    .map_err(|error| anyhow::anyhow!("Invalid H.264 access unit: {error:?}"))?;
                Ok(SessionProfile::H264(*profile))
            }
        }
    }
}
