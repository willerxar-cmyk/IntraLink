//! v12 bounded viewer profile negotiation. This is a local resource policy,
//! not a hardware capability probe or a guarantee of actual frame rate.
use crate::{
    capabilities::CodecCapabilities,
    desktop::{CONFIG_BYTES, CodecMode, REQUEST_BYTES, VideoConfig},
};

pub const ALPN: &[u8] = b"intralink/view/12-bounded";
const PREFIX: &[u8; 16] = b"INTRALINK/12VIEW";
pub const LIMIT_BYTES: usize = 3;
pub const SELECTION_BYTES: usize = 2 * CONFIG_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiverLimits {
    height: u16,
    fps: u8,
}

impl ReceiverLimits {
    pub const MAX: Self = Self {
        height: 1080,
        fps: 60,
    };

    pub const fn new(height: u16, fps: u32) -> Option<Self> {
        if !matches!(height, 540 | 720 | 1080) || fps == 0 || fps > 60 {
            None
        } else {
            Some(Self {
                height,
                fps: fps as u8,
            })
        }
    }

    pub const fn height(self) -> u16 {
        self.height
    }
    pub const fn fps(self) -> u32 {
        self.fps as u32
    }

    pub const fn encode(self) -> [u8; LIMIT_BYTES] {
        let [hi, lo] = self.height.to_be_bytes();
        [hi, lo, self.fps]
    }

    pub const fn decode(bytes: [u8; LIMIT_BYTES]) -> Option<Self> {
        Self::new(u16::from_be_bytes([bytes[0], bytes[1]]), bytes[2] as u32)
    }

    /// Both ends derive exactly the same result from the approved Host
    /// ceiling and authenticated viewer offer. The viewer cannot increase
    /// Host privileges, duration or any resource limit.
    pub fn select(self, host: VideoConfig, offered: CodecCapabilities) -> Option<VideoConfig> {
        let codec = CodecCapabilities::through(host.mode()).select(offered)?;
        VideoConfig::new(
            self.height.min(host.height() as u16),
            self.fps().min(host.fps()),
            host.quality(),
            host.seconds(),
        )
        .map(|config| config.with_mode(codec))
    }

    /// Independent local defense before encoder initialization.
    pub fn within_approval(host: VideoConfig, selected: VideoConfig) -> bool {
        selected.height() <= host.height()
            && selected.fps() <= host.fps()
            && selected.quality() == host.quality()
            && selected.seconds() == host.seconds()
            && CodecCapabilities::through(host.mode()).contains(selected.mode())
    }
}

pub fn request(mode: CodecMode) -> [u8; REQUEST_BYTES] {
    let mut bytes = [0; REQUEST_BYTES];
    bytes[..16].copy_from_slice(PREFIX);
    bytes[16] = CodecCapabilities::through(mode).bits();
    bytes
}

pub fn parse_request(bytes: &[u8]) -> Option<CodecCapabilities> {
    if bytes.len() != REQUEST_BYTES || &bytes[..16] != PREFIX {
        return None;
    }
    CodecCapabilities::from_bits(bytes[16])
}

pub fn selection(host: VideoConfig, selected: VideoConfig) -> [u8; SELECTION_BYTES] {
    let mut bytes = [0; SELECTION_BYTES];
    bytes[..CONFIG_BYTES].copy_from_slice(&host.encode());
    bytes[CONFIG_BYTES..].copy_from_slice(&selected.encode());
    bytes
}

/// Validate both advertised approval and exact chosen intersection. Reject
/// a malicious sender raising or arbitrarily lowering the negotiated profile.
pub fn validate_selection(
    limits: ReceiverLimits,
    offered: CodecCapabilities,
    bytes: [u8; SELECTION_BYTES],
) -> Option<VideoConfig> {
    let host = VideoConfig::decode(bytes[..CONFIG_BYTES].try_into().ok()?)?;
    let selected = VideoConfig::decode(bytes[CONFIG_BYTES..].try_into().ok()?)?;
    (limits.select(host, offered) == Some(selected)).then_some(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_limits_and_exact_intersection() {
        for height in [540, 720, 1080] {
            for fps in 1..=60 {
                let limits = ReceiverLimits::new(height, fps).unwrap();
                assert_eq!(ReceiverLimits::decode(limits.encode()), Some(limits));
                for host_height in [540, 720, 1080] {
                    for host_fps in [1, 15, 30, 60] {
                        let host = VideoConfig::new(host_height, host_fps, 75, 300)
                            .unwrap()
                            .with_mode(CodecMode::Compact);
                        let caps = CodecCapabilities::through(CodecMode::Tiles);
                        let chosen = limits.select(host, caps).unwrap();
                        assert_eq!(chosen.height(), u32::from(height.min(host_height)));
                        assert_eq!(chosen.fps(), fps.min(host_fps));
                        assert_eq!(chosen.mode(), CodecMode::Tiles);
                        assert!(ReceiverLimits::within_approval(host, chosen));
                        assert_eq!(
                            validate_selection(limits, caps, selection(host, chosen)),
                            Some(chosen)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn malformed_limits_and_changed_selection_are_rejected() {
        for height in [0, 539, 541, 719, 721, 1081, 2160, u16::MAX] {
            assert!(ReceiverLimits::new(height, 60).is_none());
        }
        for fps in [0, 61, 255, 256, u32::MAX] {
            assert!(ReceiverLimits::new(1080, fps).is_none());
        }
        let limits = ReceiverLimits::new(720, 20).unwrap();
        let caps = CodecCapabilities::through(CodecMode::Compact);
        let host = VideoConfig::new(1080, 60, 75, 60)
            .unwrap()
            .with_mode(CodecMode::Compact);
        let good = limits.select(host, caps).unwrap();
        for bad in [
            host,
            good.with_mode(CodecMode::Jpeg),
            VideoConfig::new(540, 20, 75, 60)
                .unwrap()
                .with_mode(CodecMode::Compact),
            VideoConfig::new(720, 21, 75, 60)
                .unwrap()
                .with_mode(CodecMode::Compact),
            VideoConfig::new(720, 20, 76, 60)
                .unwrap()
                .with_mode(CodecMode::Compact),
            VideoConfig::new(720, 20, 75, 61)
                .unwrap()
                .with_mode(CodecMode::Compact),
        ] {
            assert!(validate_selection(limits, caps, selection(host, bad)).is_none());
        }
        assert!(!ReceiverLimits::within_approval(good, host));
        assert_eq!(
            parse_request(&request(CodecMode::Tiles)),
            Some(CodecCapabilities::through(CodecMode::Tiles))
        );
        assert!(CodecCapabilities::parse_request(&request(CodecMode::Tiles)).is_none());
        assert!(parse_request(&request(CodecMode::Tiles)[..16]).is_none());
    }
}
