//! Fixed, locally generated NVENC fixture for the restricted decoder gate.
//! No file, network packet or caller-supplied bitstream is accepted.
use anyhow::{Result, ensure};

use crate::nvdec::Decoder;

const CLIP: &[u8] = include_bytes!("../examples/fixtures/headless-bands-1080p.h264");
const PACKET_SIZES: [usize; 6] = [895, 2642, 2563, 866, 2581, 2579];

/// Decode exact known native access units, verify immediate callbacks,
/// sampled NV12 colors, order and teardown, without creating a presenter.
pub fn verify_restricted_headless_fixture(luid: [u8; 8]) -> Result<()> {
    crate::harden_decoder_worker(true)?;
    ensure!(
        PACKET_SIZES.iter().sum::<usize>() == CLIP.len(),
        "Embedded fixture boundaries changed"
    );
    let mut decoder = Decoder::with_luid(luid, (1920, 1080))?;
    let mut offset = 0;
    for size in PACKET_SIZES {
        let end = offset + size;
        decoder.submit_immediate(&CLIP[offset..end])?;
        offset = end;
    }
    eprintln!("{}", decoder.finish_summary()?);
    Ok(())
}
