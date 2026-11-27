//! Portable RGBA source to existing bounded software desktop records.
//! Native JPEG encoding remains a local platform adapter; no new wire format.
use crate::{
    canvas::letterbox_rgba,
    desktop::{MAX_FRAME_BYTES, VideoConfig},
    media::{self, Delta, DeltaError, JPEG_TAG, MAX_DELTA_BYTES},
    portable_motion::MotionPolicy,
    quality::{AdaptiveQuality, ReceiptPressure},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Repeat,
    Encoded,
    Jpeg { quality: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Dimensions,
    State,
    Tile(DeltaError),
    Jpeg,
}

/// A single session-owned canvas pair. Native images never escape the
/// synchronous capture call. The prior RGB base advances only after the
/// caller supplies a complete bounded record; a failed send closes the epoch.
pub struct Encoder {
    config: VideoConfig,
    current: Vec<u8>,
    previous: Vec<u8>,
    quality: AdaptiveQuality,
    motion: MotionPolicy,
    dense: bool,
    lossy_base: bool,
    refresh_attempted: bool,
    initialized: bool,
    pending: Option<Decision>,
    deltas: u16,
    key_bytes: usize,
}

impl Encoder {
    pub fn new(config: VideoConfig) -> Self {
        let len = (config.width() * config.height() * 3) as usize;
        Self {
            config,
            current: vec![0; len],
            previous: vec![0; len],
            quality: AdaptiveQuality::new(config),
            motion: MotionPolicy::new(config),
            dense: false,
            lossy_base: false,
            refresh_attempted: false,
            initialized: false,
            pending: None,
            deltas: 0,
            key_bytes: 0,
        }
    }

    pub const fn config(&self) -> VideoConfig {
        self.config
    }

    pub const fn has_frame(&self) -> bool {
        self.initialized
    }

    pub fn prepare(
        &mut self,
        rgba: &[u8],
        pressure: ReceiptPressure,
        output: &mut Vec<u8>,
    ) -> Result<Decision, Error> {
        output.clear();
        if self.pending.is_some() {
            return Err(Error::State);
        }
        let size = (self.config.width(), self.config.height());
        if !letterbox_rgba(rgba, size, &mut self.current, size) {
            return Err(Error::Dimensions);
        }
        let unchanged = self.initialized && self.current == self.previous;
        self.quality.observe(pressure, !unchanged);
        self.dense = self.initialized
            && !unchanged
            && MotionPolicy::dense(
                &self.previous,
                &self.current,
                size.0 as usize,
                size.1 as usize,
            );
        // Tiles compare original source canvases, not the lossy JPEG
        // reconstructed by the receiver. Restore an exact full base once
        // when a lossy epoch becomes quiet, even if the source is unchanged.
        // Failed refreshes are bounded: retry after a new JPEG or dense epoch.
        if self.dense {
            self.refresh_attempted = false;
        }
        let refresh = self.lossy_base
            && !self.dense
            && !self.refresh_attempted
            && self.config.mode().supports_compact();
        if refresh {
            self.refresh_attempted = true;
            if media::encode_keyframe(self.config, &self.current, output, MAX_DELTA_BYTES)
                .map_err(Error::Tile)?
            {
                self.pending = Some(Decision::Encoded);
                return Ok(Decision::Encoded);
            }
        }
        if unchanged {
            return Ok(Decision::Repeat);
        }
        let direct_jpeg = self.motion.prefer_jpeg(self.dense);
        if !direct_jpeg
            && self.config.mode().supports_tiles()
            && self.initialized
            && self.deltas < 120
        {
            let budget = self
                .motion
                .delta_budget(self.key_bytes.clamp(32 * 1024, MAX_DELTA_BYTES), self.dense);
            if media::encode_delta(self.config, &self.previous, &self.current, output, budget)
                .map_err(Error::Tile)?
                == Delta::Encoded
            {
                self.pending = Some(Decision::Encoded);
                return Ok(Decision::Encoded);
            }
        }
        let decision = if !direct_jpeg
            && !refresh
            && self.config.mode().supports_compact()
            && media::encode_keyframe(self.config, &self.current, output, MAX_DELTA_BYTES)
                .map_err(Error::Tile)?
        {
            Decision::Encoded
        } else {
            output.clear();
            Decision::Jpeg {
                quality: self.motion.quality(self.quality.current(), self.dense),
            }
        };
        self.pending = Some(decision);
        Ok(decision)
    }

    /// Native JPEG callback supplies a complete locally encoded image. This
    /// checks the framing/budget only; the existing bounded receiver validates
    /// actual JPEG dimensions and decode. No untrusted JPEG is decoded here.
    pub fn commit(
        &mut self,
        decision: Decision,
        output: &mut Vec<u8>,
        jpeg: Option<&[u8]>,
    ) -> Result<(), Error> {
        if self.pending != Some(decision) || decision == Decision::Repeat {
            output.clear();
            return Err(Error::State);
        }
        let keyframe = match decision {
            Decision::Jpeg { .. } => {
                let bytes = jpeg.ok_or(Error::Jpeg)?;
                if !(4..MAX_FRAME_BYTES).contains(&bytes.len())
                    || !bytes.starts_with(&[0xff, 0xd8])
                    || !bytes.ends_with(&[0xff, 0xd9])
                {
                    output.clear();
                    return Err(Error::Jpeg);
                }
                output.clear();
                output.push(JPEG_TAG);
                output.extend_from_slice(bytes);
                true
            }
            Decision::Encoded => {
                if jpeg.is_some() || !(2..=MAX_FRAME_BYTES).contains(&output.len()) {
                    output.clear();
                    return Err(Error::State);
                }
                output[0] == media::LOSSLESS_KEYFRAME_TAG
            }
            Decision::Repeat => unreachable!(),
        };
        self.motion.committed(
            matches!(decision, Decision::Jpeg { .. }),
            output.len(),
            self.dense,
        );
        if keyframe {
            self.lossy_base = matches!(decision, Decision::Jpeg { .. });
            self.refresh_attempted = false;
            self.deltas = 0;
            self.key_bytes = output.len();
        } else {
            self.deltas += 1;
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.initialized = true;
        self.pending = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::CodecMode;

    fn config(mode: CodecMode) -> VideoConfig {
        VideoConfig::new(540, 60, 75, 15).unwrap().with_mode(mode)
    }

    #[test]
    fn compact_initial_delta_repeat_and_exact_canvas() {
        let cfg = config(CodecMode::Compact);
        let mut encoder = Encoder::new(cfg);
        let n = (cfg.width() * cfg.height()) as usize;
        let mut rgba = [17, 22, 33, 255].repeat(n);
        let mut data = Vec::new();
        let d = encoder
            .prepare(&rgba, ReceiptPressure::Low, &mut data)
            .unwrap();
        assert_eq!(d, Decision::Encoded);
        assert!(!encoder.has_frame());
        encoder.commit(d, &mut data, None).unwrap();
        let mut canvas = Vec::new();
        media::apply_keyframe(cfg, &data, &mut canvas).unwrap();
        assert!(canvas.iter().all(|p| *p == 0x111621));
        rgba[..4].copy_from_slice(&[201, 33, 9, 255]);
        let d = encoder
            .prepare(&rgba, ReceiptPressure::Low, &mut data)
            .unwrap();
        encoder.commit(d, &mut data, None).unwrap();
        media::apply_delta(cfg, &data, &mut canvas).unwrap();
        assert_eq!(canvas[0], 0xc92109);
        assert_eq!(
            encoder.prepare(&rgba, ReceiptPressure::Low, &mut data),
            Ok(Decision::Repeat)
        );
        assert!(data.is_empty());
    }

    #[test]
    fn jpeg_fallback_budget_and_failure_do_not_publish_base() {
        let cfg = config(CodecMode::Jpeg);
        let mut encoder = Encoder::new(cfg);
        let rgba = [1, 2, 3, 255].repeat((cfg.width() * cfg.height()) as usize);
        let mut data = vec![1, 2, 3];
        assert_eq!(
            encoder.prepare(&rgba[..4], ReceiptPressure::Neutral, &mut data),
            Err(Error::Dimensions)
        );
        assert!(data.is_empty());
        let d = encoder
            .prepare(&rgba, ReceiptPressure::High, &mut data)
            .unwrap();
        assert!(matches!(d, Decision::Jpeg { .. }));
        assert_eq!(
            encoder.commit(d, &mut data, Some(&[1, 2, 3, 4])),
            Err(Error::Jpeg)
        );
        assert!(!encoder.has_frame());
        assert!(data.is_empty());
        // This epoch must terminate after an invalid native result.
        assert_eq!(
            encoder.prepare(&rgba, ReceiptPressure::Neutral, &mut data),
            Err(Error::State)
        );
    }

    #[test]
    fn compact_restores_exact_base_after_lossy_motion() {
        let cfg = config(CodecMode::Compact);
        let n = (cfg.width() * cfg.height()) as usize;
        let mut encoder = Encoder::new(cfg);
        let mut data = Vec::new();
        let mut rgba = vec![0; n * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let x = i % cfg.width() as usize;
            let y = i / cfg.width() as usize;
            px.copy_from_slice(&[
                (x * 3 + y * 2) as u8,
                (x + y * 5) as u8,
                (x * 2 + y * 3) as u8,
                255,
            ]);
        }
        let d = encoder
            .prepare(&rgba, ReceiptPressure::Neutral, &mut data)
            .unwrap();
        assert!(matches!(d, Decision::Jpeg { .. }));
        encoder
            .commit(d, &mut data, Some(&[0xff, 0xd8, 0xff, 0xd9]))
            .unwrap();
        for px in rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&[20, 36, 54, 255]);
        }
        let d = encoder
            .prepare(&rgba, ReceiptPressure::Neutral, &mut data)
            .unwrap();
        assert_eq!(d, Decision::Encoded);
        assert_eq!(data[0], media::LOSSLESS_KEYFRAME_TAG);
        encoder.commit(d, &mut data, None).unwrap();
        let mut pixels = Vec::new();
        media::apply_keyframe(cfg, &data, &mut pixels).unwrap();
        assert!(pixels.iter().all(|p| *p == 0x142436));
        assert_eq!(
            encoder.prepare(&rgba, ReceiptPressure::Neutral, &mut data),
            Ok(Decision::Repeat)
        );
    }

    #[test]
    fn jpeg_record_uses_existing_tag_and_requires_complete_framing() {
        let cfg = config(CodecMode::Jpeg);
        let mut encoder = Encoder::new(cfg);
        let rgba = [1, 2, 3, 255].repeat((cfg.width() * cfg.height()) as usize);
        let mut data = Vec::new();
        let d = encoder
            .prepare(&rgba, ReceiptPressure::Neutral, &mut data)
            .unwrap();
        let marker_only = [0xff, 0xd8, 0xff, 0xd9]; // framing test, not a decodable JPEG
        encoder.commit(d, &mut data, Some(&marker_only)).unwrap();
        assert_eq!(data, [JPEG_TAG, 0xff, 0xd8, 0xff, 0xd9]);
        assert_eq!(
            encoder.prepare(&rgba, ReceiptPressure::Neutral, &mut data),
            Ok(Decision::Repeat)
        );
    }
}
