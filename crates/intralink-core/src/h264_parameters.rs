//! Restricted AVC parameter-set parser for the experimental v9 profile.
//! Bounded RBSP parsing in safe Rust. Not a complete slice/CABAC validator.
use super::{Error, Profile};

const MAX_PARAMETER_BYTES: usize = 512;

struct Bits {
    bytes: [u8; MAX_PARAMETER_BYTES],
    len: usize,
    pos: usize,
}

impl Bits {
    fn new(ebsp: &[u8]) -> Result<Self, Error> {
        if ebsp.is_empty() || ebsp.len() > MAX_PARAMETER_BYTES {
            return Err(Error::ParameterSets);
        }
        let mut result = Self {
            bytes: [0; MAX_PARAMETER_BYTES],
            len: 0,
            pos: 0,
        };
        let mut zeros = 0;
        let mut index = 0;
        while index < ebsp.len() {
            let byte = ebsp[index];
            index += 1;
            if zeros == 2 {
                match byte {
                    0..=2 => return Err(Error::ParameterSets),
                    3 => {
                        if ebsp.get(index).is_none_or(|next| *next > 3) {
                            return Err(Error::ParameterSets);
                        }
                        zeros = 0;
                        continue;
                    }
                    _ => {}
                }
            }
            result.bytes[result.len] = byte;
            result.len += 1;
            zeros = if byte == 0 { zeros + 1 } else { 0 };
        }
        Ok(result)
    }

    fn read(&mut self, count: usize) -> Result<u32, Error> {
        if count > 32 || self.pos + count > self.len * 8 {
            return Err(Error::ParameterSets);
        }
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | u32::from((self.bytes[self.pos / 8] >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Ok(value)
    }

    fn flag(&mut self) -> Result<bool, Error> {
        Ok(self.read(1)? == 1)
    }

    fn ue(&mut self, max: u32) -> Result<u32, Error> {
        let mut zeros = 0;
        while !self.flag()? {
            zeros += 1;
            // This restricted profile has no valid unbounded Exp-Golomb
            // field. Refuse large prefixes before shifts or allocations.
            if zeros > 16 {
                return Err(Error::ParameterSets);
            }
        }
        let value = (1u32 << zeros) - 1 + self.read(zeros)?;
        if value > max {
            return Err(Error::ParameterSets);
        }
        Ok(value)
    }

    fn se(&mut self, min: i32, max: i32) -> Result<i32, Error> {
        let code = self.ue(128)?;
        let half = code.div_ceil(2) as i32;
        let value = if code & 1 == 1 { half } else { -half };
        if !(min..=max).contains(&value) {
            return Err(Error::ParameterSets);
        }
        Ok(value)
    }

    fn expect(&mut self, count: usize, expected: u32) -> Result<(), Error> {
        if self.read(count)? == expected {
            Ok(())
        } else {
            Err(Error::ParameterSets)
        }
    }

    fn expect_ue(&mut self, expected: u32) -> Result<(), Error> {
        if self.ue(expected)? == expected {
            Ok(())
        } else {
            Err(Error::ParameterSets)
        }
    }

    fn finish(&mut self) -> Result<(), Error> {
        self.expect(1, 1)?; // rbsp_stop_one_bit
        while !self.pos.is_multiple_of(8) {
            self.expect(1, 0)?;
        }
        if self.pos != self.len * 8 {
            return Err(Error::ParameterSets);
        }
        Ok(())
    }
}

/// Bounded observation of a progressive 8-bit 4:2:0 SPS prefix.
/// This deliberately does not validate VUI, slice syntax, DPB conformance
/// or the complete RBSP. It is not a decoder admission check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpsInfo {
    pub profile_idc: u8,
    pub level_idc: u8,
    pub constraints: u8,
    pub width: u16,
    pub height: u16,
    pub references: u8,
    pub poc_type: u8,
}

pub(super) fn observe_sps(nal: &[u8]) -> Result<SpsInfo, Error> {
    if nal.len() > MAX_PARAMETER_BYTES || nal.first().is_none_or(|b| b & 0x9f != 7 || b & 0x60 == 0)
    {
        return Err(Error::ParameterSets);
    }
    let mut bits = Bits::new(nal.get(1..).ok_or(Error::ParameterSets)?)?;
    let profile_idc = bits.read(8)? as u8;
    let constraints = bits.read(8)? as u8;
    let level_idc = bits.read(8)? as u8;
    if !matches!(profile_idc, 66 | 77 | 100)
        || constraints & 3 != 0
        || !matches!(
            level_idc,
            10 | 11
                | 12
                | 13
                | 20
                | 21
                | 22
                | 30
                | 31
                | 32
                | 40
                | 41
                | 42
                | 50
                | 51
                | 52
                | 60
                | 61
                | 62
        )
    {
        return Err(Error::ParameterSets);
    }
    bits.ue(31)?; // SPS id
    if profile_idc == 100 {
        bits.expect_ue(1)?; // 4:2:0
        bits.expect_ue(0)?; // 8-bit luma
        bits.expect_ue(0)?; // 8-bit chroma
        bits.expect(1, 0)?; // no transform bypass
        bits.expect(1, 0)?; // custom scaling lists are not observed by this parser
    }
    bits.ue(12)?; // log2_max_frame_num_minus4
    let poc_type = bits.ue(2)? as u8;
    match poc_type {
        0 => {
            bits.ue(12)?;
        }
        2 => {}
        _ => return Err(Error::ParameterSets), // type 1 not supported here
    }
    let references = bits.ue(16)? as u8;
    bits.flag()?; // gaps_in_frame_num_value_allowed_flag
    let coded_width = (bits.ue(119)? + 1) * 16;
    let coded_height = (bits.ue(119)? + 1) * 16;
    bits.expect(1, 1)?; // progressive only
    bits.flag()?; // direct_8x8_inference_flag
    let crop = if bits.flag()? {
        [bits.ue(959)?, bits.ue(959)?, bits.ue(959)?, bits.ue(959)?]
    } else {
        [0; 4]
    };
    let width = coded_width
        .checked_sub(2 * (crop[0] + crop[1]))
        .ok_or(Error::ParameterSets)?;
    let height = coded_height
        .checked_sub(2 * (crop[2] + crop[3]))
        .ok_or(Error::ParameterSets)?;
    if !(64..=1920).contains(&width)
        || !(64..=1920).contains(&height)
        || width.min(height) > 1080
        || coded_width.min(coded_height) > 1088
    {
        return Err(Error::ParameterSets);
    }
    bits.flag()?; // VUI present flag only; no color/timing inference
    Ok(SpsInfo {
        profile_idc,
        level_idc,
        constraints,
        width: width as u16,
        height: height as u16,
        references,
        poc_type,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sps {
    pub id: u32,
    pub references: u32,
}

pub(super) fn sps(nal: &[u8], profile: Profile) -> Result<Sps, Error> {
    let mut bits = Bits::new(nal.get(1..).ok_or(Error::ParameterSets)?)?;
    bits.expect(8, 100)?; // High
    let constraints = bits.read(8)?;
    if constraints & 3 != 0 {
        return Err(Error::ParameterSets);
    }
    let level = bits.read(8)?;
    // Validate level bounds against macroblock throughput, frame size and DPB.
    let (max_mbps, max_fs, max_dpb, max_bitrate) = match level {
        30 => (40_500, 1_620, 8_100, 12_500_000),
        31 => (108_000, 3_600, 18_000, 17_500_000),
        32 => (216_000, 5_120, 20_480, 25_000_000),
        40 => (245_760, 8_192, 32_768, 25_000_000),
        41 => (245_760, 8_192, 32_768, 62_500_000),
        42 => (522_240, 8_704, 34_816, 62_500_000),
        _ => return Err(Error::ParameterSets),
    };
    let id = bits.ue(31)?;
    bits.expect_ue(1)?; // 4:2:0, no separate color plane
    bits.expect_ue(0)?; // 8-bit luma
    bits.expect_ue(0)?; // 8-bit chroma
    bits.expect(1, 0)?; // no transform bypass
    bits.expect(1, 0)?; // no custom scaling matrices
    let _frame_num_bits = bits.ue(12)? + 4;
    bits.expect_ue(2)?; // POC type 2; no reordering
    let references = bits.ue(4)?;
    if references == 0 {
        return Err(Error::ParameterSets);
    }
    bits.expect(1, 0)?; // no frame_num gaps
    let mb_width = bits.ue(119)? + 1;
    let mb_height = bits.ue(67)? + 1;
    bits.expect(1, 1)?; // frame_mbs_only_flag; no interlace
    bits.expect(1, 1)?; // direct_8x8_inference_flag
    let crop = if bits.flag()? {
        [bits.ue(8)?, bits.ue(8)?, bits.ue(8)?, bits.ue(8)?]
    } else {
        [0; 4]
    };
    let width = mb_width * 16;
    let height = mb_height * 16;
    // Only the minimal right/bottom alignment crop is permitted for 4:2:0.
    if width != profile.width().div_ceil(16) * 16
        || height != profile.height().div_ceil(16) * 16
        || crop
            != [
                0,
                (width - profile.width()) / 2,
                0,
                (height - profile.height()) / 2,
            ]
    {
        return Err(Error::ParameterSets);
    }
    let macroblocks = mb_width * mb_height;
    if macroblocks > max_fs
        || macroblocks * profile.fps() > max_mbps
        || macroblocks * references > max_dpb
        || profile.bitrate() > max_bitrate
    {
        return Err(Error::ParameterSets);
    }

    bits.expect(1, 1)?; // VUI mandatory
    bits.expect(1, 1)?; // aspect_ratio_info_present_flag
    bits.expect(8, 1)?; // square pixels
    bits.expect(1, 0)?; // no overscan metadata
    bits.expect(1, 1)?; // video_signal_type_present_flag
    bits.expect(3, 5)?; // unspecified video format, SDR metadata below
    bits.expect(1, 0)?; // limited range
    bits.expect(1, 1)?; // color description mandatory
    bits.expect(8, 1)?; // BT.709 primaries
    bits.expect(8, 1)?; // BT.709 transfer
    bits.expect(8, 1)?; // BT.709 matrix
    bits.expect(1, 1)?; // explicit chroma location
    bits.expect_ue(0)?;
    bits.expect_ue(0)?;
    bits.expect(1, 1)?; // timing mandatory
    let tick = u64::from(bits.read(32)?);
    let scale = u64::from(bits.read(32)?);
    if tick == 0 || scale != tick * 2 * u64::from(profile.fps()) {
        return Err(Error::ParameterSets);
    }
    bits.expect(1, 1)?; // fixed_frame_rate
    bits.expect(1, 0)?; // no NAL HRD
    bits.expect(1, 0)?; // no VCL HRD
    bits.expect(1, 0)?; // no pic_struct
    bits.expect(1, 0)?; // no additional bitstream restrictions
    bits.finish()?;
    Ok(Sps { id, references })
}

pub(super) fn pps(nal: &[u8], sps: Sps) -> Result<u32, Error> {
    let mut bits = Bits::new(nal.get(1..).ok_or(Error::ParameterSets)?)?;
    let id = bits.ue(255)?;
    bits.expect_ue(sps.id)?;
    bits.expect(1, 1)?; // CABAC
    bits.expect(1, 0)?; // no bottom-field picture order
    bits.expect_ue(0)?; // no flexible macroblock ordering
    bits.ue(sps.references - 1)?; // bounded active L0 references
    bits.expect_ue(0)?; // no L1 reference use
    bits.expect(1, 0)?; // no weighted prediction
    bits.expect(2, 0)?;
    bits.se(-26, 25)?; // pic_init_qp_minus26
    bits.se(-26, 25)?;
    bits.se(-12, 12)?;
    bits.expect(1, 1)?; // deblocking control
    bits.expect(1, 0)?; // unconstrained intra prediction
    bits.expect(1, 0)?; // no redundant pictures
    bits.expect(1, 1)?; // 8x8 transform
    bits.expect(1, 0)?; // no custom picture scaling matrices
    bits.se(-12, 12)?;
    bits.finish()?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Exact SPS/PPS bytes from native NVENC 60 FPS fixtures on RTX 3080 Ti.
    const SPS540: &[u8] = &[
        0x67, 0x64, 0x00, 0x20, 0xac, 0x2b, 0x20, 0x1e, 0x02, 0x2f, 0xde, 0x02, 0xd4, 0x04, 0x04,
        0x07, 0xc0, 0x00, 0x00, 0xfa, 0x00, 0x00, 0x75, 0x30, 0x21,
    ];
    const SPS720: &[u8] = &[
        0x67, 0x64, 0x00, 0x20, 0xac, 0x2b, 0x20, 0x0a, 0x00, 0xb7, 0x60, 0x2d, 0x40, 0x40, 0x40,
        0x7c, 0x00, 0x00, 0x0f, 0xa0, 0x00, 0x07, 0x53, 0x02, 0x10,
    ];
    const SPS1080: &[u8] = &[
        0x67, 0x64, 0x00, 0x2a, 0xac, 0x2b, 0x20, 0x0f, 0x00, 0x44, 0xfc, 0xb8, 0x0b, 0x50, 0x10,
        0x10, 0x1f, 0x00, 0x00, 0x03, 0x03, 0xe8, 0x00, 0x01, 0xd4, 0xc0, 0x84,
    ];
    const PPS: &[u8] = &[0x68, 0xeb, 0x8f, 0x2c];

    #[test]
    fn native_parameter_sets_match_exact_profiles() {
        for (height, bytes) in [(540, SPS540), (720, SPS720), (1080, SPS1080)] {
            let profile = Profile::new(height, 60, 3, 8_000_000).unwrap();
            let parsed = sps(bytes, profile).unwrap();
            assert_eq!(
                parsed,
                Sps {
                    id: 0,
                    references: 3
                }
            );
            assert_eq!(pps(PPS, parsed), Ok(0));
            assert!(sps(bytes, Profile::new(height, 30, 3, 8_000_000).unwrap()).is_err());
            let wrong = if height == 540 { 720 } else { 540 };
            assert!(sps(bytes, Profile::new(wrong, 60, 3, 8_000_000).unwrap()).is_err());
        }
    }

    #[test]
    fn rbsp_escape_rules_and_exp_golomb_limits() {
        let parsed = Bits::new(&[0, 0, 3, 1, 0x80]).unwrap();
        assert_eq!(&parsed.bytes[..parsed.len], &[0, 0, 1, 0x80]);
        for invalid in [
            &[0, 0, 3][..],
            &[0, 0, 3, 4],
            &[0, 0, 0],
            &[0, 0, 1],
            &[0, 0, 2],
        ] {
            assert!(Bits::new(invalid).is_err());
        }
        // The byte after an emulation-prevention byte must be <= 3.
        // The previous fixture incorrectly put 0x80 after the second 0x03.
        assert!(Bits::new(&[0, 0, 3, 0, 0, 3, 0x80]).is_err());
        let repeated = Bits::new(&[0, 0, 3, 0, 0, 3, 0, 0x80]).unwrap();
        assert_eq!(&repeated.bytes[..repeated.len], &[0, 0, 0, 0, 0, 0x80]);
        for next in 0..=3 {
            let escaped = Bits::new(&[0, 0, 3, next, 0x80]).unwrap();
            assert_eq!(&escaped.bytes[..escaped.len], &[0, 0, next, 0x80]);
        }
        assert!(Bits::new(&[0, 0, 3, 0, 1]).unwrap().ue(u32::MAX).is_err());
    }

    #[test]
    fn truncated_parameter_sets_and_extra_rbsp_are_rejected() {
        let profile = Profile::new(1080, 60, 3, 8_000_000).unwrap();
        for end in 0..SPS1080.len() {
            assert!(sps(&SPS1080[..end], profile).is_err());
        }
        let parsed = sps(SPS1080, profile).unwrap();
        for end in 0..PPS.len() {
            assert!(pps(&PPS[..end], parsed).is_err());
        }
        let mut extra = SPS1080.to_vec();
        extra.push(0xff);
        assert!(sps(&extra, profile).is_err());
        let mut wrong_level = SPS1080.to_vec();
        wrong_level[3] = 32;
        assert!(sps(&wrong_level, profile).is_err());
        let mut invalid = PPS.to_vec();
        invalid.push(0xff);
        assert!(pps(&invalid, parsed).is_err());
    }
}
