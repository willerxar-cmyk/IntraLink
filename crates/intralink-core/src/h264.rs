//! Isolated v9 H.264 wire profile. This is a bounded transport envelope, not
//! a complete AVC syntax validator or a sandbox for a native decoder.
use crate::desktop::{MAX_FPS, MAX_FRAME_BYTES, MAX_IN_FLIGHT_RECORDS, MAX_SECONDS};

#[path = "h264_packetizer.rs"]
mod packetizer;
#[path = "h264_parameters.rs"]
mod parameters;
pub use packetizer::{Framing, Packetizer, inspect_nal_types};

pub const ALPN: &[u8] = b"intralink/view/9-h264";
pub const REQUEST: &[u8; 17] = b"INTRALINK/9 VIEW\x03";
pub const CONFIG_BYTES: usize = 12;
pub const AU_HEADER: usize = 8;
pub const MAX_ACCESS_UNIT: usize = MAX_FRAME_BYTES - AU_HEADER;
const PROFILE: u8 = 1; // High, progressive, 8-bit 4:2:0, BT.709 limited
const MAX_NALS: usize = 32;
const MAX_PARAMETER_SET: usize = 512;
const MAX_SEI: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    height: u16,
    fps: u8,
    seconds: u16,
    bitrate: u32,
}

impl Profile {
    pub fn new(height: u16, fps: u32, seconds: u64, bitrate: u32) -> Option<Self> {
        if !matches!(height, 540 | 720 | 1080)
            || !(1..=MAX_FPS).contains(&fps)
            || !(1..=MAX_SECONDS).contains(&seconds)
            || !(500_000..=20_000_000).contains(&bitrate)
        {
            return None;
        }
        Some(Self {
            height,
            fps: fps as u8,
            seconds: seconds as u16,
            bitrate,
        })
    }

    pub const fn width(self) -> u32 {
        self.height as u32 * 16 / 9
    }
    pub const fn height(self) -> u32 {
        self.height as u32
    }
    pub const fn fps(self) -> u32 {
        self.fps as u32
    }
    pub const fn seconds(self) -> u64 {
        self.seconds as u64
    }
    pub const fn bitrate(self) -> u32 {
        self.bitrate
    }
    pub const fn max_frames(self) -> usize {
        self.seconds as usize * self.fps as usize
    }

    pub fn encode(self) -> [u8; CONFIG_BYTES] {
        let mut bytes = [0; CONFIG_BYTES];
        bytes[..2].copy_from_slice(&self.height.to_be_bytes());
        bytes[2] = self.fps;
        bytes[3] = PROFILE;
        bytes[4..6].copy_from_slice(&self.seconds.to_be_bytes());
        bytes[6] = MAX_IN_FLIGHT_RECORDS as u8;
        bytes[8..].copy_from_slice(&self.bitrate.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: [u8; CONFIG_BYTES]) -> Option<Self> {
        if bytes[3] != PROFILE || bytes[6] != MAX_IN_FLIGHT_RECORDS as u8 || bytes[7] != 0 {
            return None;
        }
        Self::new(
            u16::from_be_bytes([bytes[0], bytes[1]]),
            u32::from(bytes[2]),
            u64::from(u16::from_be_bytes([bytes[4], bytes[5]])),
            u32::from_be_bytes(bytes[8..].try_into().ok()?),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Envelope,
    Size,
    Sequence,
    Nal,
    ParameterSets,
    Keyframe,
    Gop,
}

#[derive(Clone, Copy, Debug)]
pub struct AccessUnit<'a> {
    pub sequence: u32,
    pub keyframe: bool,
    pub annex_b: &'a [u8],
}

pub fn wrap(
    sequence: u32,
    keyframe: bool,
    annex_b: &[u8],
    output: &mut Vec<u8>,
) -> Result<(), Error> {
    if sequence == 0 || annex_b.is_empty() || annex_b.len() > MAX_ACCESS_UNIT {
        return Err(Error::Size);
    }
    begin_record(sequence, keyframe, output);
    output.extend_from_slice(annex_b);
    Ok(())
}

fn begin_record(sequence: u32, keyframe: bool, output: &mut Vec<u8>) {
    output.clear();
    output.extend_from_slice(&[0x48, 1, u8::from(keyframe), 0]);
    output.extend_from_slice(&sequence.to_be_bytes());
}

pub fn unwrap(bytes: &[u8]) -> Result<AccessUnit<'_>, Error> {
    if !(AU_HEADER + 1..=MAX_FRAME_BYTES).contains(&bytes.len()) {
        return Err(Error::Size);
    }
    if bytes[0..2] != [0x48, 1] || bytes[2] > 1 || bytes[3] != 0 {
        return Err(Error::Envelope);
    }
    let sequence = u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| Error::Envelope)?);
    if sequence == 0 {
        return Err(Error::Sequence);
    }
    Ok(AccessUnit {
        sequence,
        keyframe: bytes[2] == 1,
        annex_b: &bytes[AU_HEADER..],
    })
}

/// Locate the next Annex B start code in linear time, accepting 3 or 4 bytes.
/// Interior zero runs are handled as Annex B trailing zero bytes.
fn start_code(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let i = bytes.get(from..)?.windows(3).position(|w| w == [0, 0, 1])? + from;
    let start = if i > from && bytes[i - 1] == 0 {
        i - 1
    } else {
        i
    };
    Some((start, i + 3))
}

fn ue(bytes: &[u8], position: &mut usize) -> Option<u32> {
    let bit = |i: usize| bytes.get(i / 8).map(|b| (b >> (7 - i % 8)) & 1);
    let mut zeros = 0;
    while bit(*position)? == 0 {
        *position += 1;
        zeros += 1;
        if zeros > 8 {
            return None;
        }
    }
    *position += 1;
    let mut value = 1u32;
    for _ in 0..zeros {
        value = (value << 1) | u32::from(bit(*position)?);
        *position += 1;
    }
    Some(value - 1)
}

#[derive(Default)]
struct Inspection<'a> {
    sps: Option<&'a [u8]>,
    pps: Option<&'a [u8]>,
    pps_id: Option<u32>,
    keyframe: bool,
}

fn inspect(au: AccessUnit<'_>, profile: Profile) -> Result<Inspection<'_>, Error> {
    let bytes = au.annex_b;
    let (first, mut begin) = start_code(bytes, 0).ok_or(Error::Nal)?;
    if first != 0 {
        return Err(Error::Nal);
    }
    let mut result = Inspection::default();
    let mut aud = None;
    let mut picture = false;
    let mut count = 0usize;
    let mut parsed_sps = None;
    let mut pps_id = None;
    loop {
        let next = start_code(bytes, begin);
        let mut end = next.map_or(bytes.len(), |(offset, _)| offset);
        while end > begin && bytes[end - 1] == 0 {
            end -= 1;
        }
        let nal = bytes.get(begin..end).ok_or(Error::Nal)?;
        count += 1;
        if nal.len() < 2 || count > MAX_NALS || nal[0] & 0x80 != 0 {
            return Err(Error::Nal);
        }
        let kind = nal[0] & 31;
        let reference = nal[0] & 0x60;
        if count == 1 && kind != 9 {
            return Err(Error::Nal);
        }
        if picture && kind != 12 {
            return Err(Error::Nal);
        }
        match kind {
            9 if count == 1 && reference == 0 && nal.len() == 2 => {
                // AUD primary_pic_type 0 (I) or 1 (I/P), followed by rbsp stop bit.
                if !matches!(nal[1], 0x10 | 0x30) {
                    return Err(Error::Nal);
                }
                aud = Some(nal[1]);
            }
            7 if au.keyframe && result.sps.is_none() && result.pps.is_none() => {
                if reference == 0
                    || !(5..=MAX_PARAMETER_SET).contains(&nal.len())
                    || nal[1] != 100
                    || nal[2] & 3 != 0
                    || !(10..=42).contains(&nal[3])
                {
                    return Err(Error::ParameterSets);
                }
                parsed_sps = Some(parameters::sps(nal, profile)?);
                result.sps = Some(nal);
            }
            8 if au.keyframe && result.sps.is_some() && result.pps.is_none() => {
                if reference == 0 || nal.len() > MAX_PARAMETER_SET {
                    return Err(Error::ParameterSets);
                }
                let parsed = parsed_sps.ok_or(Error::ParameterSets)?;
                pps_id = Some(parameters::pps(nal, parsed)?);
                result.pps = Some(nal);
            }
            6 if reference == 0 && nal.len() <= MAX_SEI => {}
            1 | 5 => {
                if picture || aud.is_none() {
                    return Err(Error::Nal);
                }
                let mut pos = 0;
                let first_mb = ue(&nal[1..], &mut pos).ok_or(Error::Nal)?;
                let slice = ue(&nal[1..], &mut pos).ok_or(Error::Nal)?;
                let id = ue(&nal[1..], &mut pos).ok_or(Error::Nal)?;
                if first_mb != 0
                    || id > 255
                    || !matches!(slice, 0 | 2 | 5 | 7)
                    || (kind == 5
                        && (reference == 0 || !matches!(slice, 2 | 7) || aud != Some(0x10)))
                    || (matches!(slice, 0 | 5) && aud != Some(0x30))
                {
                    return Err(Error::Nal);
                }
                if kind == 5 && Some(id) != pps_id {
                    return Err(Error::ParameterSets);
                }
                result.pps_id = Some(id);
                result.keyframe = kind == 5;
                picture = true;
            }
            12 if reference == 0 && picture => {}
            _ => return Err(Error::Nal),
        }
        if let Some((_, after)) = next {
            begin = after;
        } else {
            break;
        }
    }
    if !picture || result.keyframe != au.keyframe {
        return Err(Error::Keyframe);
    }
    if au.keyframe && (result.sps.is_none() || result.pps.is_none()) {
        return Err(Error::ParameterSets);
    }
    Ok(result)
}

/// A session accepts only the next coded picture. No-update and verified-repeat
/// records do not advance this sequence. On a gap or incompatible headers,
/// close the session rather than presenting a potentially corrupted reference.
pub struct StreamState {
    profile: Profile,
    next: u32,
    last_idr: Option<u32>,
    sps: Vec<u8>,
    pps: Vec<u8>,
    pps_id: Option<u32>,
}

impl StreamState {
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            next: 1,
            last_idr: None,
            sps: Vec::new(),
            pps: Vec::new(),
            pps_id: None,
        }
    }

    pub fn accept<'a>(&mut self, bytes: &'a [u8]) -> Result<AccessUnit<'a>, Error> {
        let au = unwrap(bytes)?;
        if au.sequence != self.next || au.sequence as usize > self.profile.max_frames() {
            return Err(Error::Sequence);
        }
        let inspected = inspect(au, self.profile)?;
        if au.keyframe {
            let sps = inspected.sps.ok_or(Error::ParameterSets)?;
            let pps = inspected.pps.ok_or(Error::ParameterSets)?;
            if self.last_idr.is_some() && (sps != self.sps || pps != self.pps) {
                return Err(Error::ParameterSets);
            }
            if self.last_idr.is_none() {
                self.sps.extend_from_slice(sps);
                self.pps.extend_from_slice(pps);
                self.pps_id = inspected.pps_id;
            }
            self.last_idr = Some(au.sequence);
        } else {
            let last = self.last_idr.ok_or(Error::Keyframe)?;
            if inspected.pps_id != self.pps_id {
                return Err(Error::ParameterSets);
            }
            // Require an IDR within one second of coded pictures, not wire
            // record sequence; a static/no-update interval cannot consume GOP.
            if au.sequence - last >= self.profile.fps() {
                return Err(Error::Gop);
            }
        }
        self.next += 1;
        Ok(au)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Structural fixtures only; not claimed to be decodable AVC streams.
    pub const IDR: &[u8] = &[
        0x00, 0x00, 0x01, 0x09, 0x10, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x20, 0xac, 0x2b, 0x20,
        0x1e, 0x02, 0x2f, 0xde, 0x02, 0xd4, 0x04, 0x04, 0x07, 0xc0, 0x00, 0x00, 0xfa, 0x00, 0x00,
        0x05, 0xdc, 0x21, 0x00, 0x00, 0x01, 0x68, 0xeb, 0x8f, 0x2c, 0x00, 0x00, 0x01, 0x65, 0xb8,
    ];
    pub const P: &[u8] = &[0, 0, 1, 0x09, 0x30, 0, 0, 1, 0x61, 0xe0];
    fn record(sequence: u32, key: bool, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        wrap(sequence, key, data, &mut out).unwrap();
        out
    }
    fn profile() -> Profile {
        Profile::new(540, 3, 10, 8_000_000).unwrap()
    }

    #[test]
    fn profile_and_wire_version_are_exact() {
        let p = profile();
        assert_eq!(Profile::decode(p.encode()), Some(p));
        for (index, value) in [(3, 0), (6, 9), (7, 1), (2, 0), (2, 61)] {
            let mut b = p.encode();
            b[index] = value;
            assert!(Profile::decode(b).is_none());
        }
        assert!(Profile::new(2160, 60, 10, 8_000_000).is_none());
        assert!(Profile::new(1080, 60, 10, 20_000_001).is_none());
        assert!(crate::desktop::parse_request(REQUEST).is_none());
    }

    #[test]
    fn envelope_rejects_limits_and_unknown_flags() {
        let mut bytes = record(1, true, IDR);
        for (i, value) in [(0, 0), (1, 2), (2, 2), (3, 1)] {
            let previous = bytes[i];
            bytes[i] = value;
            assert!(unwrap(&bytes).is_err());
            bytes[i] = previous;
        }
        assert!(unwrap(&bytes[..AU_HEADER]).is_err());
        assert!(wrap(1, true, &vec![0; MAX_ACCESS_UNIT + 1], &mut bytes).is_err());
        assert!(wrap(0, true, IDR, &mut bytes).is_err());
    }

    #[test]
    fn state_rejects_missing_base_gaps_and_gop_expiration_atomically() {
        let mut state = StreamState::new(profile());
        assert_eq!(
            state.accept(&record(1, false, P)).unwrap_err(),
            Error::Keyframe
        );
        assert!(state.accept(&record(1, true, IDR)).is_ok());
        assert!(state.accept(&record(3, false, P)).is_err());
        assert!(state.accept(&record(2, false, P)).is_ok());
        assert!(state.accept(&record(2, false, P)).is_err());
        assert!(state.accept(&record(3, false, P)).is_ok());
        assert_eq!(state.accept(&record(4, false, P)).unwrap_err(), Error::Gop);
        assert!(state.accept(&record(4, true, IDR)).is_ok());
    }

    #[test]
    fn rejected_parameter_change_does_not_advance_state() {
        let mut state = StreamState::new(profile());
        assert!(state.accept(&record(1, true, IDR)).is_ok());
        let mut changed = IDR.to_vec();
        changed[11] = 31;
        assert_eq!(
            state.accept(&record(2, true, &changed)).unwrap_err(),
            Error::ParameterSets
        );
        assert!(state.accept(&record(2, true, IDR)).is_ok());
    }

    #[test]
    fn rejects_missing_headers_b_slices_and_multiple_pictures() {
        for (key, data) in [
            (true, P),
            (true, &[0, 0, 1, 9, 0x10, 0, 0, 1, 0x65, 0xb8][..]),
            (false, &[0, 0, 1, 9, 0x30, 0, 0, 1, 0x61, 0xa0][..]), // B slice
            (false, &[0, 0, 1, 9, 0x30, 0, 0, 1, 0xe1, 0xe0][..]),
            (
                false,
                &[0, 0, 1, 9, 0x30, 0, 0, 1, 0x61, 0xe0, 0, 0, 1, 0x61, 0xe0][..],
            ),
            (false, &[0, 0, 1, 9, 0x30, 0, 0, 1][..]),
        ] {
            let mut state = StreamState::new(profile());
            assert!(state.accept(&record(1, true, IDR)).is_ok());
            assert!(state.accept(&record(2, key, data)).is_err());
            assert!(state.accept(&record(2, false, P)).is_ok());
        }
    }

    #[test]
    fn every_truncation_and_mutation_is_bounded() {
        let original = record(1, true, IDR);
        for end in 0..original.len() {
            let _ = StreamState::new(profile()).accept(&original[..end]);
        }
        for i in 0..original.len() {
            for value in [0, 1, 0x7f, 0x80, 0xff] {
                let mut data = original.clone();
                data[i] = value;
                let _ = StreamState::new(profile()).accept(&data);
            }
        }
    }
}
