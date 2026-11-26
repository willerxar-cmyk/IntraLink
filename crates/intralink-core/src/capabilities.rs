//! Capability negotiation for the software-only v11 desktop protocol.
//! The local codec ceiling authorizes a subset of compiled CPU codecs.
use crate::desktop::{CodecMode, REQUEST_BYTES};

pub const ALPN: &[u8] = b"intralink/view/11-negotiated";
const PREFIX: &[u8; 16] = b"INTRALINK/11VIEW";
const KNOWN: u8 = 0b111;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodecCapabilities(u8);

impl CodecCapabilities {
    /// Each codec is implemented by the common software encoder and decoder.
    /// The explicit ceiling is local permission, not a GPU/vendor probe.
    pub const fn through(ceiling: CodecMode) -> Self {
        Self((1 << (ceiling as u8 + 1)) - 1)
    }

    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits == 0 || bits & !KNOWN != 0 {
            None
        } else {
            Some(Self(bits))
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, codec: CodecMode) -> bool {
        self.0 & (1 << codec as u8) != 0
    }

    /// Prefer the richest mutually enabled software mode. Never select an
    /// unadvertised capability or an isolated native diagnostic codec.
    pub fn select(self, peer: Self) -> Option<CodecMode> {
        [CodecMode::Compact, CodecMode::Tiles, CodecMode::Jpeg]
            .into_iter()
            .find(|&codec| self.contains(codec) && peer.contains(codec))
    }

    pub fn request(self) -> [u8; REQUEST_BYTES] {
        let mut bytes = [0; REQUEST_BYTES];
        bytes[..16].copy_from_slice(PREFIX);
        bytes[16] = self.bits();
        bytes
    }

    pub fn parse_request(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != REQUEST_BYTES || &bytes[..16] != PREFIX {
            return None;
        }
        Self::from_bits(bytes[16])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_masks_are_bounded_and_selection_is_symmetric() {
        for bits in 0..=u8::MAX {
            assert_eq!(
                CodecCapabilities::from_bits(bits).is_some(),
                (1..=KNOWN).contains(&bits)
            );
        }
        for host in 1..=KNOWN {
            for viewer in 1..=KNOWN {
                let a = CodecCapabilities::from_bits(host).unwrap();
                let b = CodecCapabilities::from_bits(viewer).unwrap();
                let expected = (0..3)
                    .rev()
                    .find(|i| (host & viewer) & (1 << i) != 0)
                    .and_then(CodecMode::decode);
                assert_eq!(a.select(b), expected);
                assert_eq!(a.select(b), b.select(a));
            }
        }
    }

    #[test]
    fn local_codec_ceiling_and_request_roundtrip() {
        for (mode, mask) in [
            (CodecMode::Jpeg, 1),
            (CodecMode::Tiles, 3),
            (CodecMode::Compact, 7),
        ] {
            let caps = CodecCapabilities::through(mode);
            assert_eq!(caps.bits(), mask);
            let request = caps.request();
            assert_eq!(CodecCapabilities::parse_request(&request), Some(caps));
            assert_eq!(crate::desktop::parse_request(&request), None);
            assert_eq!(CodecCapabilities::parse_request(&request[..16]), None);
            let mut bad = request;
            bad[16] = 8;
            assert_eq!(CodecCapabilities::parse_request(&bad), None);
            bad = request;
            bad[0] ^= 1;
            assert_eq!(CodecCapabilities::parse_request(&bad), None);
        }
    }
}
