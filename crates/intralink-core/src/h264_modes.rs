//! Local exact-mode intersection for independently exercised AVC codecs.
//! No wire format, permission grant or assertion of decoder safety.
use super::SpsInfo;

pub const MAX_MODES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Bt709Limited,
    Bt601Limited,
    Bt709Full,
}

/// Each entry describes an exact operating point previously exercised by
/// the local encoder or decoder. An SPS observation alone does not qualify
/// a mode. In particular, color, frame rate, bitrate and decoder support
/// must be measured or explicitly verified outside this data structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    sps: SpsInfo,
    color: Color,
    fps: u8,
    bitrate: u32,
}

impl Mode {
    pub fn new(sps: SpsInfo, color: Color, fps: u8, bitrate: u32) -> Option<Self> {
        if !matches!(sps.profile_idc, 66 | 77 | 100)
            || sps.constraints & 3 != 0
            || !matches!(sps.level_idc, 30 | 31 | 32 | 40 | 41 | 42)
            || !matches!(sps.poc_type, 0 | 2)
            || sps.references > 16
            || !(64..=1920).contains(&sps.width)
            || !(64..=1920).contains(&sps.height)
            || sps.width.min(sps.height) > 1080
            || !sps.width.is_multiple_of(2)
            || !sps.height.is_multiple_of(2)
            || !(1..=60).contains(&fps)
            || !(500_000..=20_000_000).contains(&bitrate)
        {
            return None;
        }
        // These are resource ceilings, not a complete AVC level validator.
        // Use visible dimensions rounded up to macroblocks; the actual coded
        // dimensions/DPB and parameter sets require separate validation.
        let mb = u32::from(sps.width).div_ceil(16) * u32::from(sps.height).div_ceil(16);
        let (max_mbps, max_fs, max_dpb, base_bitrate) = match sps.level_idc {
            30 => (40_500, 1_620, 8_100, 10_000_000),
            31 => (108_000, 3_600, 18_000, 14_000_000),
            32 => (216_000, 5_120, 20_480, 20_000_000),
            40 => (245_760, 8_192, 32_768, 20_000_000),
            41 => (245_760, 8_192, 32_768, 50_000_000),
            42 => (522_240, 8_704, 34_816, 50_000_000),
            _ => return None,
        };
        let max_bitrate = if sps.profile_idc == 100 {
            base_bitrate * 5 / 4
        } else {
            base_bitrate
        };
        if mb > max_fs
            || mb * u32::from(fps) > max_mbps
            || mb * u32::from(sps.references) > max_dpb
            || bitrate > max_bitrate
        {
            return None;
        }
        Some(Self {
            sps,
            color,
            fps,
            bitrate,
        })
    }

    pub const fn sps(self) -> SpsInfo {
        self.sps
    }
    pub const fn color(self) -> Color {
        self.color
    }
    pub const fn fps(self) -> u8 {
        self.fps
    }
    pub const fn bitrate(self) -> u32 {
        self.bitrate
    }
}

/// Local receiver policy independent of the claimed encoder modes. Width
/// and height are orientation-specific; do not silently rotate or upscale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    width: u16,
    height: u16,
    fps: u8,
    bitrate: u32,
}

impl Limits {
    pub fn new(width: u16, height: u16, fps: u8, bitrate: u32) -> Option<Self> {
        if !(64..=1920).contains(&width)
            || !(64..=1920).contains(&height)
            || !(1..=60).contains(&fps)
            || !(500_000..=20_000_000).contains(&bitrate)
        {
            return None;
        }
        Some(Self {
            width,
            height,
            fps,
            bitrate,
        })
    }

    pub const fn dimensions(self) -> (u16, u16) {
        (self.width, self.height)
    }

    pub const fn fps(self) -> u8 {
        self.fps
    }

    pub const fn bitrate(self) -> u32 {
        self.bitrate
    }

    pub fn permits(self, mode: Mode) -> bool {
        mode.sps.width <= self.width
            && mode.sps.height <= self.height
            && mode.fps <= self.fps
            && mode.bitrate <= self.bitrate
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferError {
    Length,
    Duplicate,
}

fn validate(modes: &[Mode]) -> Result<(), OfferError> {
    if modes.len() > MAX_MODES {
        return Err(OfferError::Length);
    }
    for (i, mode) in modes.iter().enumerate() {
        if modes[..i].contains(mode) {
            return Err(OfferError::Duplicate);
        }
    }
    Ok(())
}

/// Preserve the producer's explicit approved preference order; receiver order
/// cannot change it. Match all fields exactly and apply both local ceilings.
/// No synthesized profile, silent fallback, capability grant or wire change.
pub fn select_exact(
    producer: &[Mode],
    producer_limits: Limits,
    receiver: &[Mode],
    receiver_limits: Limits,
) -> Result<Option<Mode>, OfferError> {
    validate(producer)?;
    validate(receiver)?;
    Ok(producer.iter().copied().find(|mode| {
        producer_limits.permits(*mode) && receiver_limits.permits(*mode) && receiver.contains(mode)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::h264::{Framing, inspect_sps};

    fn mode(w: u16, h: u16, fps: u8, color: Color) -> Mode {
        Mode::new(
            SpsInfo {
                profile_idc: 100,
                level_idc: 42,
                constraints: 0,
                width: w,
                height: h,
                references: 3,
                poc_type: 2,
            },
            color,
            fps,
            8_000_000,
        )
        .unwrap()
    }
    fn limits() -> Limits {
        Limits::new(1920, 1920, 60, 12_000_000).unwrap()
    }

    #[test]
    fn exact_orientation_color_rate_and_local_ceilings() {
        let full = mode(1920, 1080, 60, Color::Bt709Limited);
        let reduced = mode(1280, 720, 30, Color::Bt709Limited);
        let portrait = mode(1080, 1920, 60, Color::Bt709Limited);
        let color = mode(1920, 1080, 60, Color::Bt601Limited);
        let offer = [full, reduced, portrait];
        assert_eq!(
            select_exact(&offer, limits(), &[portrait, full, reduced], limits()),
            Ok(Some(full))
        );
        assert_eq!(select_exact(&offer, limits(), &[color], limits()), Ok(None));
        assert_eq!(
            select_exact(
                &offer,
                limits(),
                &[portrait],
                Limits::new(1920, 1080, 60, 12_000_000).unwrap()
            ),
            Ok(None)
        );
        assert_eq!(
            select_exact(
                &offer,
                limits(),
                &[full, reduced],
                Limits::new(1920, 1080, 30, 12_000_000).unwrap()
            ),
            Ok(Some(reduced))
        );
        assert_eq!(
            select_exact(
                &offer,
                Limits::new(1920, 1080, 30, 12_000_000).unwrap(),
                &[full],
                limits()
            ),
            Ok(None)
        );
        assert_eq!(
            select_exact(
                &offer,
                limits(),
                &[full],
                Limits::new(1920, 1920, 60, 6_000_000).unwrap()
            ),
            Ok(None)
        );
    }

    #[test]
    fn malformed_offers_and_impossible_operating_points() {
        let full = mode(1920, 1080, 60, Color::Bt709Limited);
        assert_eq!(
            select_exact(&[full, full], limits(), &[full], limits()),
            Err(OfferError::Duplicate)
        );
        assert_eq!(
            select_exact(&[full; MAX_MODES + 1], limits(), &[], limits()),
            Err(OfferError::Length)
        );
        let mut sps = full.sps();
        sps.level_idc = 40;
        assert!(Mode::new(sps, full.color(), 60, full.bitrate()).is_none());
        sps = full.sps();
        sps.references = 16;
        assert!(Mode::new(sps, full.color(), 30, full.bitrate()).is_none());
        sps = full.sps();
        sps.width = 1921;
        assert!(Mode::new(sps, full.color(), 30, full.bitrate()).is_none());
    }

    #[test]
    fn actual_native_sps_is_observed_and_exact_profile_selects() {
        let sps = [
            0, 0, 1, 0x67, 0x64, 0x00, 0x2a, 0xac, 0x2b, 0x20, 0x0f, 0x00, 0x44, 0xfc, 0xb8, 0x0b,
            0x50, 0x10, 0x10, 0x1f, 0x00, 0x00, 0x03, 0x03, 0xe8, 0x00, 0x01, 0xd4, 0xc0, 0x84,
        ];
        let observed = inspect_sps(Framing::AnnexB, &sps).unwrap();
        assert_eq!(
            (
                observed.width,
                observed.height,
                observed.profile_idc,
                observed.level_idc,
                observed.references,
                observed.poc_type
            ),
            (1920, 1080, 100, 42, 3, 2)
        );
        let full = Mode::new(observed, Color::Bt709Limited, 60, 8_000_000).unwrap();
        assert_eq!(
            select_exact(&[full], limits(), &[full], limits()),
            Ok(Some(full))
        );
        for end in 0..sps.len() {
            let _ = inspect_sps(Framing::AnnexB, &sps[..end]);
        }
        let mut duplicate = sps.to_vec();
        duplicate.extend_from_slice(&sps);
        assert!(inspect_sps(Framing::AnnexB, &duplicate).is_err());
    }
}
