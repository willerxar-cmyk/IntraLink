//! Normalize locally produced AVC into the existing restricted v9 envelope.
//! No decoder, networking, permissions or platform-specific dependency.
use super::{
    Error, MAX_ACCESS_UNIT, MAX_FRAME_BYTES, MAX_NALS, MAX_PARAMETER_SET, Profile, StreamState,
    begin_record, parameters, start_code,
};

/// The producer must know its framing. Never guess from the first payload bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    AnnexB,
    LengthPrefixed4,
}

struct Nals<'a> {
    items: [&'a [u8]; MAX_NALS],
    len: usize,
}

impl<'a> Nals<'a> {
    fn as_slice(&self) -> &[&'a [u8]] {
        &self.items[..self.len]
    }
}

fn nals(framing: Framing, data: &[u8]) -> Result<Nals<'_>, Error> {
    if data.is_empty() || data.len() > MAX_ACCESS_UNIT {
        return Err(Error::Size);
    }
    let mut result = Nals {
        items: [&[]; MAX_NALS],
        len: 0,
    };
    let mut offset = 0;
    while offset < data.len() {
        if result.len == MAX_NALS {
            return Err(Error::Nal);
        }
        let nal = match framing {
            Framing::AnnexB => {
                let (start, begin) = start_code(data, offset).ok_or(Error::Nal)?;
                if start != offset {
                    return Err(Error::Nal);
                }
                let next = start_code(data, begin).map_or(data.len(), |(start, _)| start);
                let mut end = next;
                while end > begin && data[end - 1] == 0 {
                    end -= 1;
                }
                offset = next;
                &data[begin..end]
            }
            Framing::LengthPrefixed4 => {
                let header = data.get(offset..offset + 4).ok_or(Error::Nal)?;
                let size = u32::from_be_bytes(header.try_into().map_err(|_| Error::Nal)?) as usize;
                let begin = offset + 4;
                let end = begin.checked_add(size).ok_or(Error::Size)?;
                let nal = data.get(begin..end).ok_or(Error::Nal)?;
                if nal.windows(3).any(|w| w == [0, 0, 1]) {
                    return Err(Error::Nal);
                }
                offset = end;
                nal
            }
        };
        if nal.len() < 2 || nal[0] & 0x80 != 0 {
            return Err(Error::Nal);
        }
        result.items[result.len] = nal;
        result.len += 1;
    }
    Ok(result)
}

fn append(output: &mut Vec<u8>, nal: &[u8]) -> Result<(), Error> {
    if output.len() + 3 + nal.len() > MAX_FRAME_BYTES {
        return Err(Error::Size);
    }
    output.extend_from_slice(&[0, 0, 1]);
    output.extend_from_slice(nal);
    Ok(())
}

fn check_config(profile: Profile, sps: &[u8], pps: &[u8]) -> Result<(), Error> {
    if sps.len() > MAX_PARAMETER_SET
        || pps.len() > MAX_PARAMETER_SET
        || sps.first().is_none_or(|b| b & 0x9f != 7 || b & 0x60 == 0)
        || pps.first().is_none_or(|b| b & 0x9f != 8 || b & 0x60 == 0)
    {
        return Err(Error::ParameterSets);
    }
    parameters::pps(pps, parameters::sps(sps, profile)?)?;
    Ok(())
}

/// One ordered encoder epoch with immutable parameter sets. The existing v9
/// validator remains the final authority. A lost coded picture or unsupported
/// encoder configuration requires ending the diagnostic, not continuing with
/// missing references. Only a caller-owned reusable output buffer is needed.
pub struct Packetizer {
    state: StreamState,
    sps: Vec<u8>,
    pps: Vec<u8>,
}

impl Packetizer {
    pub fn new(profile: Profile) -> Self {
        Self {
            state: StreamState::new(profile),
            sps: Vec::new(),
            pps: Vec::new(),
        }
    }

    /// Accept separate encoder CSD buffers (one SPS and one PPS). Accepting
    /// configuration does not create a coded picture or authorize transport.
    pub fn configure(&mut self, framing: Framing, sps: &[u8], pps: &[u8]) -> Result<(), Error> {
        if sps.len() > MAX_PARAMETER_SET + 4 || pps.len() > MAX_PARAMETER_SET + 4 {
            return Err(Error::Size);
        }
        let s = nals(framing, sps)?;
        let p = nals(framing, pps)?;
        let (s, p) = (s.as_slice(), p.as_slice());
        if s.len() != 1 || p.len() != 1 {
            return Err(Error::ParameterSets);
        }
        check_config(self.state.profile, s[0], p[0])?;
        if !self.sps.is_empty() && (self.sps != s[0] || self.pps != p[0]) {
            return Err(Error::ParameterSets);
        }
        if self.sps.is_empty() {
            self.sps.extend_from_slice(s[0]);
            self.pps.extend_from_slice(p[0]);
        }
        Ok(())
    }

    /// Output exactly one existing v9 record. The caller must submit each
    /// successful record in order, with bounded demand/receipt credit. No
    /// partial records, implicit framing detection or arbitrary format changes.
    /// On failure no sequence or accepted parameter sets advance.
    pub fn packetize(
        &mut self,
        framing: Framing,
        keyframe: bool,
        data: &[u8],
        output: &mut Vec<u8>,
    ) -> Result<(), Error> {
        output.clear();
        let result = self.packetize_inner(framing, keyframe, data, output);
        if result.is_err() {
            output.clear();
        }
        result
    }

    fn packetize_inner(
        &mut self,
        framing: Framing,
        keyframe: bool,
        data: &[u8],
        output: &mut Vec<u8>,
    ) -> Result<(), Error> {
        if self.state.next as usize > self.state.profile.max_frames() {
            return Err(Error::Sequence);
        }
        let parsed = nals(framing, data)?;
        let units = parsed.as_slice();
        let mut sps = None;
        let mut pps = None;
        let mut aud = None;
        let mut payload_start = units.len();
        let mut picture = false;
        for (index, &nal) in units.iter().enumerate() {
            match nal[0] & 31 {
                9 if index == 0 => aud = Some(nal),
                7 if keyframe && payload_start == units.len() && sps.is_none() && pps.is_none() => {
                    sps = Some(nal)
                }
                8 if keyframe && payload_start == units.len() && sps.is_some() && pps.is_none() => {
                    pps = Some(nal)
                }
                1 | 5 if !picture => {
                    picture = true;
                    payload_start = payload_start.min(index);
                }
                6 if !picture => payload_start = payload_start.min(index),
                12 if picture => {}
                _ => return Err(Error::Nal),
            }
        }
        if !picture || sps.is_some() != pps.is_some() {
            return Err(Error::Keyframe);
        }
        if let (Some(s), Some(p)) = (sps, pps) {
            check_config(self.state.profile, s, p)?;
            if !self.sps.is_empty() && (self.sps != s || self.pps != p) {
                return Err(Error::ParameterSets);
            }
        }
        // The shared header helper preserves the existing v9 wire format.
        // All NAL descriptors are stack-bounded; coded bytes are copied only
        // into the caller's record buffer, with no intermediate frame copy.
        begin_record(self.state.next, keyframe, output);
        let default_aud = [9, if keyframe { 0x10 } else { 0x30 }];
        append(output, aud.unwrap_or(&default_aud))?;
        if keyframe {
            let s = sps.unwrap_or(&self.sps);
            let p = pps.unwrap_or(&self.pps);
            if s.is_empty() || p.is_empty() {
                return Err(Error::ParameterSets);
            }
            append(output, s)?;
            append(output, p)?;
        }
        for nal in &units[payload_start..] {
            append(output, nal)?;
        }
        self.state.accept(output)?;
        if self.sps.is_empty()
            && let (Some(s), Some(p)) = (sps, pps)
        {
            self.sps.extend_from_slice(s);
            self.pps.extend_from_slice(p);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::h264::{
        tests::{IDR, P},
        unwrap,
    };

    fn profile() -> Profile {
        Profile::new(540, 3, 10, 8_000_000).unwrap()
    }

    fn length_prefixed(nal: &[u8]) -> Vec<u8> {
        let mut bytes = (nal.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(nal);
        bytes
    }

    #[test]
    fn separate_csd_and_missing_aud_normalize_to_existing_wire() {
        let mut sender = Packetizer::new(profile());
        let mut receiver = StreamState::new(profile());
        let parsed = nals(Framing::AnnexB, IDR).unwrap();
        let units = parsed.as_slice();
        sender
            .configure(
                Framing::LengthPrefixed4,
                &length_prefixed(units[1]),
                &length_prefixed(units[2]),
            )
            .unwrap();
        let mut output = Vec::new();
        sender
            .packetize(
                Framing::LengthPrefixed4,
                true,
                &length_prefixed(units[3]),
                &mut output,
            )
            .unwrap();
        assert_eq!(receiver.accept(&output).unwrap().annex_b, IDR);
        let mut four_byte_prefix = vec![0];
        four_byte_prefix.extend_from_slice(&P[5..]);
        sender
            .packetize(Framing::AnnexB, false, &four_byte_prefix, &mut output)
            .unwrap();
        assert_eq!(receiver.accept(&output).unwrap().annex_b, P);
        sender
            .packetize(Framing::AnnexB, true, IDR, &mut output)
            .unwrap();
        assert_eq!(receiver.accept(&output).unwrap().sequence, 3);
    }

    #[test]
    fn rejection_is_atomic_and_cannot_poison_config_or_sequence() {
        let mut sender = Packetizer::new(profile());
        let mut output = vec![42];
        assert!(
            sender
                .packetize(Framing::AnnexB, false, P, &mut output)
                .is_err()
        );
        assert!(output.is_empty());
        let mut invalid = IDR.to_vec();
        invalid.extend_from_slice(&[0, 0, 1, 0x65, 0xb8]);
        assert!(
            sender
                .packetize(Framing::AnnexB, true, &invalid, &mut output)
                .is_err()
        );
        assert!(sender.sps.is_empty());
        sender
            .packetize(Framing::AnnexB, true, IDR, &mut output)
            .unwrap();
        assert_eq!(unwrap(&output).unwrap().sequence, 1);
        let mut changed = IDR.to_vec();
        changed[11] = 31;
        assert!(
            sender
                .packetize(Framing::AnnexB, true, &changed, &mut output)
                .is_err()
        );
        assert!(output.is_empty());
        sender
            .packetize(Framing::AnnexB, false, P, &mut output)
            .unwrap();
        assert_eq!(unwrap(&output).unwrap().sequence, 2);
        sender
            .packetize(Framing::AnnexB, false, P, &mut output)
            .unwrap();
        assert_eq!(
            sender.packetize(Framing::AnnexB, false, P, &mut output),
            Err(Error::Gop)
        );
        sender
            .packetize(Framing::AnnexB, true, IDR, &mut output)
            .unwrap();
        assert_eq!(unwrap(&output).unwrap().sequence, 4);
    }

    #[test]
    fn malformed_framing_and_overflow_are_bounded() {
        for data in [
            &[0, 0, 0, 0][..],
            &[0, 0, 0, 5, 0x61, 0xe0],
            &[255, 255, 255, 255],
            &[0, 0, 0, 1, 0x61],
            &[0, 0, 0, 4, 0x61, 0, 0, 1],
        ] {
            assert!(nals(Framing::LengthPrefixed4, data).is_err());
        }
        assert!(nals(Framing::AnnexB, &[0, 0, 1]).is_err());
        assert!(nals(Framing::AnnexB, &[0, 0, 1, 0x61, 0xe0].repeat(MAX_NALS + 1)).is_err());
        assert!(nals(Framing::AnnexB, &vec![1; MAX_ACCESS_UNIT + 1]).is_err());
        assert_eq!(
            Packetizer::new(profile()).configure(Framing::AnnexB, &[0; MAX_PARAMETER_SET + 5], &[]),
            Err(Error::Size)
        );
        for end in 0..IDR.len() {
            let mut output = Vec::new();
            let _ = Packetizer::new(profile()).packetize(
                Framing::AnnexB,
                true,
                &IDR[..end],
                &mut output,
            );
        }
    }
}

#[cfg(test)]
mod native_fixture_tests {
    use super::*;
    use crate::h264::unwrap;

    #[test]
    fn generated_native_avc_retains_exact_nal_payloads() {
        // Previously qualified generated NVENC color bands; no user screen.
        // Exact access-unit offsets and provenance in the fixture README.
        let fixture =
            include_bytes!("../../intralink-d3d11/examples/fixtures/headless-bands-1080p.h264");
        let sizes = [895, 2642, 2563, 866, 2581, 2579];
        assert_eq!(fixture.len(), sizes.iter().sum());
        let profile = Profile::new(1080, 60, 3, 8_000_000).unwrap();
        let mut sender = Packetizer::new(profile);
        let mut receiver = StreamState::new(profile);
        let mut offset = 0;
        let mut output = Vec::new();
        for (index, size) in sizes.into_iter().enumerate() {
            let original = &fixture[offset..offset + size];
            let keyframe = matches!(index, 0 | 3);
            sender
                .packetize(Framing::AnnexB, keyframe, original, &mut output)
                .unwrap();
            let received = receiver.accept(&output).unwrap();
            assert_eq!(received.sequence as usize, index + 1);
            assert_eq!(received.keyframe, keyframe);
            let before = nals(Framing::AnnexB, original).unwrap();
            let after = nals(Framing::AnnexB, unwrap(&output).unwrap().annex_b).unwrap();
            assert_eq!(before.as_slice(), after.as_slice());
            offset += size;
        }
        assert_eq!(offset, fixture.len());
    }
}
