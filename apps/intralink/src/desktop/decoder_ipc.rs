//! Private stdio protocol for the isolated generated-window decoder.
//! No sockets, vault, arbitrary addresses or production remote AVC entry.
use std::io::{self, Read, Write};

use anyhow::{Context, Result, ensure};
use intralink_core::{
    desktop::MAX_FRAME_BYTES,
    h264::{self, Profile, StreamState},
};
use intralink_d3d11::{
    FIXTURE_BITRATE, FIXTURE_FPS, harden_decoder_worker, verify_isolated_decoder,
};

const MAGIC: &[u8; 8] = b"ILDEC001";
pub(super) const START_BYTES: usize = 24;

#[derive(Clone, Copy)]
pub(super) struct Start {
    pub profile: Profile,
    pub samples: u16,
    pub adapter: Option<u32>,
}

impl Start {
    pub fn encode(self) -> [u8; START_BYTES] {
        let mut bytes = [0; START_BYTES];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..20].copy_from_slice(&self.profile.encode());
        bytes[20..22].copy_from_slice(&self.samples.to_be_bytes());
        bytes[22] = self.adapter.map_or(255, |index| index as u8);
        bytes
    }

    fn decode(bytes: [u8; START_BYTES]) -> Result<Self> {
        ensure!(
            &bytes[..8] == MAGIC && bytes[23] == 0,
            "Invalid decoder IPC bootstrap"
        );
        let profile =
            Profile::decode(bytes[8..20].try_into()?).context("Invalid decoder IPC profile")?;
        ensure!(
            profile.fps() == FIXTURE_FPS
                && profile.bitrate() == FIXTURE_BITRATE
                && profile.seconds() == 30,
            "Unsupported decoder IPC profile"
        );
        let samples = u16::from_be_bytes([bytes[20], bytes[21]]);
        ensure!(
            (1..=180).contains(&samples),
            "Invalid decoder IPC frame budget"
        );
        let adapter = match bytes[22] {
            255 => None,
            index @ 0..=63 => Some(u32::from(index)),
            _ => anyhow::bail!("Invalid decoder IPC adapter"),
        };
        Ok(Self {
            profile,
            samples,
            adapter,
        })
    }

    pub fn new(profile: Profile, samples: u16, adapter: Option<u32>) -> Result<Self> {
        ensure!(
            adapter.is_none_or(|index| index < 64),
            "Invalid decoder adapter index"
        );
        Self::decode(
            Self {
                profile,
                samples,
                adapter,
            }
            .encode(),
        )
    }
}

fn next_packet<R: Read>(input: &mut R, state: &mut StreamState) -> Result<Option<Vec<u8>>> {
    let mut len = [0; 4];
    input
        .read_exact(&mut len)
        .context("Decoder IPC record length")?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 {
        // Explicit stop before the configured final frame; no flush/EOS.
        return Ok(None);
    }
    ensure!(
        (h264::AU_HEADER + 1..=MAX_FRAME_BYTES).contains(&len),
        "Invalid decoder IPC record size"
    );
    let mut envelope = vec![0; len];
    input
        .read_exact(&mut envelope)
        .context("Truncated decoder IPC access unit")?;
    state
        .accept(&envelope)
        .map_err(|error| anyhow::anyhow!("Rejected decoder IPC access unit: {error:?}"))?;
    envelope.drain(..h264::AU_HEADER);
    Ok(Some(envelope))
}

fn message(text: &str) -> Result<()> {
    let mut output = io::stdout().lock();
    writeln!(output, "{text}")?;
    output.flush()?;
    Ok(())
}

/// Hidden local-only executable mode. Bootstrap must arrive before native
/// initialization (the parent attaches its job before sending it). The job
/// check and independent v9 validator do not form a security sandbox.
pub fn child(restricted: bool) -> Result<()> {
    if restricted {
        eprintln!("restricted_decoder_worker_entry");
    }
    let mut input = io::stdin().lock();
    let mut bootstrap = [0; START_BYTES];
    input
        .read_exact(&mut bootstrap)
        .context("Missing decoder IPC bootstrap")?;
    let start = Start::decode(bootstrap)?;
    harden_decoder_worker(restricted)?;
    let mut state = StreamState::new(start.profile);
    let count = verify_isolated_decoder(
        start.profile.height() as u16,
        start.samples,
        start.adapter,
        || {
            message(if restricted {
                "ILDEC READY 1 RESTRICTED"
            } else {
                "ILDEC READY 1"
            })
        },
        || next_packet(&mut input, &mut state),
        |sequence| message(&format!("ILDEC ACK {sequence}")),
    )?;
    message(&format!("ILDEC DONE {count}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn profile() -> Profile {
        Profile::new(540, 60, 30, FIXTURE_BITRATE).unwrap()
    }

    #[test]
    fn bootstrap_is_exact_and_bounded() {
        let start = Start::new(profile(), 180, Some(1)).unwrap();
        let bytes = start.encode();
        let decoded = Start::decode(bytes).unwrap();
        assert_eq!(decoded.profile, profile());
        assert_eq!(decoded.samples, 180);
        assert_eq!(decoded.adapter, Some(1));
        for (index, value) in [(0, 0), (23, 1), (20, 1), (22, 64)] {
            let mut bad = bytes;
            bad[index] = value;
            assert!(Start::decode(bad).is_err());
        }
        assert!(Start::new(profile(), 0, None).is_err());
        assert!(Start::new(profile(), 181, None).is_err());
        assert!(Start::new(profile(), 60, Some(256)).is_err());
    }

    #[test]
    fn stop_and_invalid_lengths_do_not_allocate_or_decode() {
        let mut state = StreamState::new(profile());
        assert!(
            next_packet(&mut Cursor::new([0, 0, 0, 0]), &mut state)
                .unwrap()
                .is_none()
        );
        for len in [
            1u32,
            h264::AU_HEADER as u32,
            (MAX_FRAME_BYTES + 1) as u32,
            u32::MAX,
        ] {
            assert!(next_packet(&mut Cursor::new(len.to_be_bytes()), &mut state).is_err());
        }
        assert!(next_packet(&mut Cursor::new([0, 0, 0]), &mut state).is_err());
        let mut truncated = 10u32.to_be_bytes().to_vec();
        truncated.extend_from_slice(&[0x48, 1, 0]);
        assert!(next_packet(&mut Cursor::new(truncated), &mut state).is_err());
    }
}
