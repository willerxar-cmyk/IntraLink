//! Offline local generated Android fixture admission assessment. The strict
//! v9 validator is unchanged. No networking, decoder or arbitrary playback.
use intralink_core::h264::{self, Framing, Packetizer, Profile};
use std::{
    error::Error,
    io::{self, Read},
};

const MAX: u64 = 16 * 1024 * 1024;

fn main() -> Result<(), Box<dyn Error>> {
    let mut bytes = Vec::new();
    io::stdin().take(MAX + 1).read_to_end(&mut bytes)?;
    if bytes.len() < 36 || bytes.len() > MAX as usize || &bytes[..8] != b"ILAVC001" {
        return Err("Invalid local generated fixture envelope".into());
    }
    let width = u16::from_be_bytes(bytes[8..10].try_into()?);
    let height = u16::from_be_bytes(bytes[10..12].try_into()?);
    let fps = u32::from_be_bytes(bytes[12..16].try_into()?);
    let bitrate = u32::from_be_bytes(bytes[16..20].try_into()?);
    if (width, height, fps, bitrate) != (1920, 1080, 60, 6_000_000) {
        return Err("Unexpected fixture configuration".into());
    }
    let profile = Profile::new(height, fps, 15, bitrate).ok_or("Invalid profile")?;
    let mut packetizer = Packetizer::new(profile);
    let mut output = Vec::new();
    let mut sps = None;
    let mut pps = None;
    let mut first_rejection = None;
    let mut frames = 0usize;
    let mut accepted = 0usize;
    let mut pts = -1i64;
    let mut end = false;
    let mut at = 20usize;
    let mut configs = 0usize;
    while at < bytes.len() {
        let header = bytes.get(at..at + 16).ok_or("Truncated record")?;
        let kind = header[0];
        let flags = header[1];
        let length = u32::from_be_bytes(header[4..8].try_into()?) as usize;
        let timestamp = i64::from_be_bytes(header[8..16].try_into()?);
        if header[2..4] != [0, 0] || flags > 1 || length > h264::MAX_ACCESS_UNIT {
            return Err("Invalid record header".into());
        }
        at += 16;
        let next = at.checked_add(length).ok_or("Length overflow")?;
        let data = bytes.get(at..next).ok_or("Truncated record data")?;
        at = next;
        if kind == 255 {
            if length != 0 || flags != 0 || timestamp != 0 || at != bytes.len() {
                return Err("Invalid fixture terminator".into());
            }
            end = true;
            break;
        }
        let mask = h264::inspect_nal_types(Framing::AnnexB, data)
            .map_err(|e| format!("Invalid local NAL framing: {e:?}"))?;
        match kind {
            1 if sps.is_none()
                && pps.is_none()
                && frames == 0
                && flags == 0
                && timestamp == 0
                && length <= 1032
                && mask == 128 =>
            {
                println!(
                    "Observed SPS: {:?}",
                    h264::inspect_sps(Framing::AnnexB, data)
                );
                sps = Some(data);
            }
            2 if pps.is_none()
                && sps.is_some()
                && frames == 0
                && flags == 0
                && timestamp == 0
                && length <= 1032
                && mask == 256 =>
            {
                pps = Some(data);
                if let Err(e) =
                    packetizer.configure(Framing::AnnexB, sps.ok_or("SPS absent")?, data)
                {
                    first_rejection = Some(format!("configuration: {e:?}"));
                }
            }
            3 if frames == 0
                && sps.is_some()
                && pps.is_some()
                && flags == 0
                && timestamp == 0
                && mask & 0x3e == 0
                && configs < 2 =>
            {
                configs += 1;
            }
            4 if sps.is_some()
                && pps.is_some()
                && timestamp > pts
                && frames < 120
                && mask & 0x3e != 0
                && (flags == 1) == (mask & 32 != 0) =>
            {
                pts = timestamp;
                frames += 1;
                if first_rejection.is_none() {
                    match packetizer.packetize(Framing::AnnexB, flags == 1, data, &mut output) {
                        Ok(()) => accepted += 1,
                        Err(e) => first_rejection = Some(format!("picture {frames}: {e:?}")),
                    }
                }
            }
            _ => return Err("Unexpected fixture record or picture order".into()),
        }
    }
    if !end || !(100..=120).contains(&frames) {
        return Err("Incomplete generated AVC fixture".into());
    }
    println!(
        "AVC_ADMISSION observed={frames} strict_v9_accepted={accepted} first_rejection={}",
        first_rejection.as_deref().unwrap_or("none")
    );
    Ok(())
}
