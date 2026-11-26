//! Portable view v10: explicit adaptive canvas geometry, no GPU dependency.
//! Only complete keyframes may establish or change the decoded canvas.
use std::{fmt, time::Duration};

use crate::{
    desktop::{self, CodecMode, MAX_FRAME_BYTES, VideoConfig},
    media::{JPEG_TAG, LOSSLESS_KEYFRAME_TAG, TILE_TAG},
    quality::ReceiptPressure,
};

pub const ALPN: &[u8] = b"intralink/view/10-adaptive";
const PREFIX: &[u8; 16] = b"INTRALINK/10VIEW";
pub const HEIGHT_BYTES: usize = 2;
const DOWN_SAMPLES: u16 = 16;
const UP_SAMPLES: u16 = 120;
// Bound resource churn even if the authenticated sender ignores local policy.
const MIN_CHANGED_BETWEEN_TRANSITIONS: u16 = DOWN_SAMPLES;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewProtocol {
    #[default]
    Fixed,
    Adaptive,
}

impl ViewProtocol {
    pub const fn alpn(self) -> &'static [u8] {
        match self {
            Self::Fixed => desktop::ALPN,
            Self::Adaptive => ALPN,
        }
    }

    pub fn request(self, mode: CodecMode) -> [u8; desktop::REQUEST_BYTES] {
        match self {
            Self::Fixed => desktop::request(mode),
            Self::Adaptive => {
                let mut bytes = [0; desktop::REQUEST_BYTES];
                bytes[..16].copy_from_slice(PREFIX);
                bytes[16] = mode as u8;
                bytes
            }
        }
    }
}

/// Retain all locally approved settings except the allowed canvas height.
pub fn config_at_height(ceiling: VideoConfig, height: u16) -> Option<VideoConfig> {
    if u32::from(height) > ceiling.height() {
        return None;
    }
    VideoConfig::new(height, ceiling.fps(), ceiling.quality(), ceiling.seconds())
        .map(|config| config.with_mode(ceiling.mode()))
}

/// Keyframe type, not proof that its compressed content is valid.
pub fn is_keyframe(mode: CodecMode, payload: &[u8]) -> bool {
    payload.len() > 1
        && match payload[0] {
            JPEG_TAG => true,
            LOSSLESS_KEYFRAME_TAG => mode.supports_compact(),
            _ => false,
        }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamError {
    InvalidGeometry,
    InvalidPayload,
    MissingKeyframe,
    TransitionTooSoon,
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidGeometry => "Adaptive frame geometry outside approved profile",
            Self::InvalidPayload => "Invalid adaptive media payload",
            Self::MissingKeyframe => "Adaptive geometry requires a complete keyframe",
            Self::TransitionTooSoon => "Adaptive geometry transition exceeds rate limit",
        };
        f.write_str(message)
    }
}

impl std::error::Error for StreamError {}

/// Both sender and receiver enforce geometry, mode and transition boundaries.
/// The software decoder separately validates the compressed pixels.
pub struct StreamState {
    ceiling: VideoConfig,
    current: Option<VideoConfig>,
    changed: u16,
}

impl StreamState {
    pub fn new(ceiling: VideoConfig) -> Self {
        Self {
            ceiling,
            current: None,
            changed: 0,
        }
    }

    pub fn accept(&mut self, height: u16, payload: &[u8]) -> Result<VideoConfig, StreamError> {
        let config = config_at_height(self.ceiling, height).ok_or(StreamError::InvalidGeometry)?;
        if !(2..=MAX_FRAME_BYTES).contains(&payload.len()) {
            return Err(StreamError::InvalidPayload);
        }
        let keyframe = is_keyframe(config.mode(), payload);
        if !(keyframe || payload[0] == TILE_TAG && config.mode().supports_tiles()) {
            return Err(StreamError::InvalidPayload);
        }
        if self.current != Some(config) {
            if !keyframe {
                return Err(StreamError::MissingKeyframe);
            }
            if self.current.is_none() && config != self.ceiling {
                return Err(StreamError::InvalidGeometry);
            }
            if self.current.is_some() && self.changed < MIN_CHANGED_BETWEEN_TRANSITIONS {
                return Err(StreamError::TransitionTooSoon);
            }
            self.current = Some(config);
            self.changed = 0;
        }
        self.changed = self.changed.saturating_add(1);
        Ok(config)
    }
}

/// Host-only hysteresis. Encode time (not native source capture time) detects
/// local CPU pressure that smaller output canvases can actually relieve.
pub struct AdaptiveResolution {
    ceiling: VideoConfig,
    height: u16,
    budget: Duration,
    high: u16,
    low: u16,
}

impl AdaptiveResolution {
    pub fn new(ceiling: VideoConfig) -> Self {
        Self {
            ceiling,
            height: ceiling.height() as u16,
            budget: Duration::from_secs_f64(1.0 / f64::from(ceiling.fps())),
            high: 0,
            low: 0,
        }
    }

    pub fn current(&self) -> u16 {
        self.height
    }

    /// Observe one changed frame after successful handoff. A neutral or
    /// background observation never drives adaptation. Recovery is slower.
    pub fn observe(
        &mut self,
        pressure: ReceiptPressure,
        encode: Duration,
        changed: bool,
    ) -> Option<VideoConfig> {
        let previous = self.height;
        if changed
            && (pressure == ReceiptPressure::High
                || (pressure == ReceiptPressure::Low && encode > self.budget))
        {
            self.low = 0;
            self.high += 1;
            if self.high == DOWN_SAMPLES {
                self.height = match self.height {
                    1080 => 720,
                    720 => 540,
                    _ => 540,
                };
                self.high = 0;
            }
        } else if changed && pressure == ReceiptPressure::Low && encode <= self.budget / 2 {
            self.high = 0;
            self.low += 1;
            if self.low == UP_SAMPLES {
                self.height = match self.height {
                    540 => 720,
                    _ => 1080,
                }
                .min(self.ceiling.height() as u16);
                self.low = 0;
            }
        } else {
            self.high = 0;
            self.low = 0;
        }
        if self.height == previous {
            None
        } else {
            config_at_height(self.ceiling, self.height)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(height: u16, mode: CodecMode) -> VideoConfig {
        VideoConfig::new(height, 60, 75, 30)
            .unwrap()
            .with_mode(mode)
    }

    #[test]
    fn request_and_alpn_are_distinct_without_fallback() {
        for mode in [CodecMode::Jpeg, CodecMode::Tiles, CodecMode::Compact] {
            assert_ne!(ViewProtocol::Adaptive.alpn(), ViewProtocol::Fixed.alpn());
            let bytes = ViewProtocol::Adaptive.request(mode);
            assert_eq!(&bytes[..16], PREFIX);
            assert_eq!(bytes[16], mode as u8);
            assert_eq!(desktop::parse_request(&bytes), None);
            assert_eq!(ViewProtocol::Fixed.request(mode), desktop::request(mode));
        }
    }

    #[test]
    fn geometry_transitions_are_keyframe_only_bounded_and_atomic() {
        let ceiling = profile(1080, CodecMode::Compact);
        let mut state = StreamState::new(ceiling);
        let key = [JPEG_TAG, 1];
        let delta = [TILE_TAG, 1];
        assert_eq!(state.accept(720, &key), Err(StreamError::InvalidGeometry));
        assert_eq!(
            state.accept(1080, &delta),
            Err(StreamError::MissingKeyframe)
        );
        assert_eq!(state.accept(1080, &key), Ok(ceiling));
        assert_eq!(state.accept(720, &key), Err(StreamError::TransitionTooSoon));
        for _ in 1..DOWN_SAMPLES {
            assert_eq!(state.accept(1080, &delta), Ok(ceiling));
        }
        assert_eq!(state.accept(719, &key), Err(StreamError::InvalidGeometry));
        assert_eq!(state.accept(720, &delta), Err(StreamError::MissingKeyframe));
        let next = config_at_height(ceiling, 720).unwrap();
        assert_eq!(state.accept(720, &[LOSSLESS_KEYFRAME_TAG, 1]), Ok(next));
        assert_eq!(
            state.accept(1080, &key),
            Err(StreamError::TransitionTooSoon)
        );
        assert_eq!(state.accept(720, &delta), Ok(next));
        for _ in 2..DOWN_SAMPLES {
            assert_eq!(state.accept(720, &delta), Ok(next));
        }
        assert_eq!(state.accept(1080, &key), Ok(ceiling));
    }

    #[test]
    fn mode_and_payload_bounds_are_enforced() {
        let mut state = StreamState::new(profile(720, CodecMode::Jpeg));
        for bad in [
            &[][..],
            &[JPEG_TAG][..],
            &[TILE_TAG, 1],
            &[LOSSLESS_KEYFRAME_TAG, 1],
            &[255, 1],
        ] {
            assert_eq!(state.accept(720, bad), Err(StreamError::InvalidPayload));
        }
        assert_eq!(
            state.accept(1080, &[JPEG_TAG, 1]),
            Err(StreamError::InvalidGeometry)
        );
        let oversized = vec![JPEG_TAG; MAX_FRAME_BYTES + 1];
        assert_eq!(
            state.accept(720, &oversized),
            Err(StreamError::InvalidPayload)
        );
        assert!(state.accept(720, &[JPEG_TAG, 1]).is_ok());
    }

    #[test]
    fn pressure_reduces_stepwise_and_recovers_to_approved_ceiling() {
        let mut policy = AdaptiveResolution::new(profile(1080, CodecMode::Compact));
        let fast = Duration::from_millis(1);
        for _ in 1..DOWN_SAMPLES {
            assert!(policy.observe(ReceiptPressure::High, fast, true).is_none());
        }
        assert_eq!(
            policy
                .observe(ReceiptPressure::High, fast, true)
                .unwrap()
                .height(),
            720
        );
        for _ in 0..DOWN_SAMPLES {
            policy.observe(ReceiptPressure::Low, Duration::from_millis(40), true);
        }
        assert_eq!(policy.current(), 540);
        for _ in 0..300 {
            policy.observe(ReceiptPressure::High, fast, true);
        }
        assert_eq!(policy.current(), 540);
        for _ in 1..UP_SAMPLES {
            assert!(policy.observe(ReceiptPressure::Low, fast, true).is_none());
        }
        assert_eq!(
            policy
                .observe(ReceiptPressure::Low, fast, true)
                .unwrap()
                .height(),
            720
        );
        for _ in 0..UP_SAMPLES {
            policy.observe(ReceiptPressure::Low, fast, true);
        }
        assert_eq!(policy.current(), 1080);
    }

    #[test]
    fn static_neutral_and_approved_ceiling_prevent_unwanted_transitions() {
        let mut policy = AdaptiveResolution::new(profile(720, CodecMode::Tiles));
        let fast = Duration::from_millis(1);
        for _ in 0..500 {
            policy.observe(ReceiptPressure::High, fast, false);
            policy.observe(ReceiptPressure::Neutral, Duration::from_secs(1), true);
            policy.observe(ReceiptPressure::Low, fast, true);
        }
        assert_eq!(policy.current(), 720);
        for _ in 0..DOWN_SAMPLES {
            policy.observe(ReceiptPressure::High, fast, true);
        }
        assert_eq!(policy.current(), 540);
        for _ in 0..500 {
            policy.observe(ReceiptPressure::Low, fast, true);
        }
        assert_eq!(policy.current(), 720);
    }
}
