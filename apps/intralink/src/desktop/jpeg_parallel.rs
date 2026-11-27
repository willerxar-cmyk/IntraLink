//! Assemble one standard baseline 4:4:4 JPEG from independently encoded
//! local MCU-row strips. Restart markers reset predictors at every join.
//! Never parse a remote or externally supplied JPEG on this code path.
use std::{io::Write, ops::Range, thread};

use anyhow::{Context, Result, ensure};
use intralink_core::{
    desktop::{MAX_FRAME_BYTES, VideoConfig},
    media::JPEG_TAG,
};

use super::jpeg::{FrameWriter, encode_dimensions, encode_rgb};

const MAX_WORKERS: usize = 4;
const MCU_EDGE: usize = 8;

#[derive(Debug)]
struct Header {
    height: usize,
    scan: usize,
    entropy: Range<usize>,
}

// Validate the exact generated baseline header and single entropy scan.
// Reject unexpected markers or changes rather than assuming compatibility.
fn header(bytes: &[u8], width: u16, height: u16) -> Result<Header> {
    ensure!(
        bytes.starts_with(&[JPEG_TAG, 0xff, 0xd8]) && bytes.ends_with(&[0xff, 0xd9]),
        "Invalid generated JPEG envelope"
    );
    let mut offset = 3;
    let mut sof_height = None;
    let mut quantization = 0usize;
    let mut huffman = 0usize;
    while offset + 4 <= bytes.len() {
        ensure!(bytes[offset] == 0xff, "Invalid generated JPEG marker");
        let marker = bytes[offset + 1];
        let len = usize::from(u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]));
        let end = offset + 2 + len;
        ensure!(
            len >= 2 && end <= bytes.len() - 2,
            "Invalid generated segment length"
        );
        let data = &bytes[offset + 4..end];
        match marker {
            0xe0 => ensure!(
                data.starts_with(b"JFIF\0"),
                "Unexpected JPEG application header"
            ),
            0xdb => quantization += 1,
            0xc4 => huffman += 1,
            0xc0 => {
                ensure!(
                    sof_height.is_none() && data.len() == 15 && data[0] == 8 && data[5] == 3,
                    "Expected one 8-bit three-component baseline frame"
                );
                ensure!(
                    data[1..3] == height.to_be_bytes() && data[3..5] == width.to_be_bytes(),
                    "Unexpected JPEG strip dimensions"
                );
                ensure!(
                    [7, 10, 13].into_iter().all(|i| data[i] == 0x11),
                    "Expected 4:4:4 sampling"
                );
                sof_height = Some(offset + 5);
            }
            0xda => {
                let height = sof_height.context("Missing baseline frame header")?;
                ensure!(
                    quantization > 0
                        && huffman > 0
                        && data.len() == 10
                        && data[0] == 3
                        && data[7..10] == [0, 63, 0],
                    "Expected single interleaved baseline scan"
                );
                let entropy = end..bytes.len() - 2;
                ensure!(!entropy.is_empty(), "Missing generated entropy scan");
                // Only byte-stuffed FF bytes can occur in this restart-free
                // trusted strip. Validate all before creating a wire record.
                for (i, &byte) in bytes[entropy.clone()].iter().enumerate() {
                    ensure!(
                        byte != 0xff || bytes.get(end + i + 1) == Some(&0),
                        "Unexpected marker in strip entropy"
                    );
                }
                return Ok(Header {
                    height,
                    scan: offset,
                    entropy,
                });
            }
            _ => anyhow::bail!("Unexpected generated JPEG marker {marker:02x}"),
        }
        offset = end;
    }
    anyhow::bail!("Missing generated JPEG scan")
}

fn compatible(reference: &[u8], first: &Header, other: &[u8], next: &Header) -> bool {
    first.height == next.height
        && first.scan == next.scan
        && first.entropy.start == next.entropy.start
        && reference[..first.height] == other[..next.height]
        && reference[first.height + 2..first.entropy.start]
            == other[next.height + 2..next.entropy.start]
}

/// Select a bounded local worker count; preserve a scalar/serial path.
/// This is a local resource policy, not a remote capability or GPU need.
fn workers() -> usize {
    if !cfg!(feature = "jpeg-parallel") {
        return 1;
    }
    match thread::available_parallelism().map_or(1, |count| count.get()) {
        0..=3 => 1,
        4..=7 => 2,
        _ => MAX_WORKERS,
    }
}

pub(super) struct JpegWorkspace {
    workers: usize,
    strips: [Vec<u8>; MAX_WORKERS],
}

impl Default for JpegWorkspace {
    fn default() -> Self {
        Self::with_workers(workers())
    }
}

impl JpegWorkspace {
    pub(super) fn with_workers(workers: usize) -> Self {
        Self {
            workers: workers.clamp(1, MAX_WORKERS),
            strips: std::array::from_fn(|_| Vec::new()),
        }
    }

    pub(super) fn count(&self) -> usize {
        self.workers
    }

    pub(super) fn encode(
        &mut self,
        config: VideoConfig,
        quality: u8,
        rgb: &[u8],
        output: &mut Vec<u8>,
    ) -> Result<()> {
        if self.workers == 1 {
            return encode_rgb(config, quality, rgb, output);
        }
        output.clear();
        let result = self.encode_strips(config, quality, rgb, output);
        if result.is_err() {
            output.clear();
        }
        result
    }

    fn encode_strips(
        &mut self,
        config: VideoConfig,
        quality: u8,
        rgb: &[u8],
        output: &mut Vec<u8>,
    ) -> Result<()> {
        ensure!(
            (35..=config.quality()).contains(&quality),
            "JPEG quality outside approved range"
        );
        let width = usize::try_from(config.width())?;
        let height = usize::try_from(config.height())?;
        ensure!(
            width.is_multiple_of(MCU_EDGE) && rgb.len() == width * height * 3,
            "Invalid parallel JPEG canvas"
        );
        let rows = height.div_ceil(MCU_EDGE);
        let strip_rows = rows.div_ceil(self.workers);
        let strip_height = strip_rows * MCU_EDGE;
        let stride = width * 3;
        let count = height.div_ceil(strip_height);
        ensure!(
            (2..=MAX_WORKERS).contains(&count),
            "Invalid local strip count"
        );
        let interval = u16::try_from(width / MCU_EDGE * strip_rows)?;
        ensure!(interval > 0, "Invalid restart interval");

        // At most four independent borrowed source slices and recycled
        // bounded strip buffers. All workers are joined before publication.
        thread::scope(|scope| -> Result<()> {
            let mut jobs = Vec::with_capacity(count);
            let mut spawn_error = None;
            for (src, dst) in rgb
                .chunks(strip_height * stride)
                .zip(&mut self.strips[..count])
            {
                // VideoConfig restricts dimensions to at most 1920x1080.
                let h = (src.len() / stride) as u16;
                let w = width as u16;
                match thread::Builder::new()
                    .spawn_scoped(scope, move || encode_dimensions(w, h, quality, src, dst))
                {
                    Ok(job) => jobs.push(job),
                    Err(error) => {
                        spawn_error = Some(error);
                        break;
                    }
                }
            }
            // Join all workers even after a spawn or encoder failure.
            let results: Vec<_> = jobs.into_iter().map(|job| job.join()).collect();
            for result in results {
                result.map_err(|_| anyhow::anyhow!("Local JPEG worker panicked"))??;
            }
            if let Some(error) = spawn_error {
                return Err(error).context("Unable to start local JPEG worker");
            }
            Ok(())
        })?;

        let strips = &self.strips[..count];
        let headers: Vec<_> = strips
            .iter()
            .enumerate()
            .map(|(index, strip)| {
                let h = (height - index * strip_height).min(strip_height);
                header(strip, width as u16, h as u16)
            })
            .collect::<Result<_>>()?;
        let first = &headers[0];
        ensure!(
            strips
                .iter()
                .zip(&headers)
                .skip(1)
                .all(|(strip, next)| compatible(&strips[0], first, strip, next)),
            "Generated JPEG strip headers differ"
        );
        let length = first.entropy.start
            + 6
            + (count - 1) * 2
            + 2
            + headers.iter().map(|h| h.entropy.len()).sum::<usize>();
        ensure!(
            length <= MAX_FRAME_BYTES,
            "Parallel JPEG exceeds frame limit"
        );
        let mut sink = FrameWriter {
            bytes: output,
            limit: MAX_FRAME_BYTES,
        };
        sink.write_all(&strips[0][..first.height])?;
        sink.write_all(&(height as u16).to_be_bytes())?;
        sink.write_all(&strips[0][first.height + 2..first.scan])?;
        sink.write_all(&[0xff, 0xdd, 0, 4])?;
        sink.write_all(&interval.to_be_bytes())?;
        sink.write_all(&strips[0][first.scan..first.entropy.start])?;
        for (index, (strip, info)) in strips.iter().zip(&headers).enumerate() {
            if index > 0 {
                sink.write_all(&[0xff, 0xd0 + (index - 1) as u8])?;
            }
            sink.write_all(&strip[info.entropy.clone()])?;
        }
        sink.write_all(&[0xff, 0xd9])?;
        ensure!(output.len() == length, "Parallel JPEG length mismatch");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::{codec::Decoder, host_calibration::dense_fixture, resize::CanvasResizer};

    #[test]
    fn restart_strips_decode_identically_to_serial_at_every_tier() -> Result<()> {
        for height in [540, 720, 1080] {
            let config = VideoConfig::new(height, 60, 90, 30).unwrap();
            let frame = dense_fixture((config.width(), config.height()))?;
            let mut rgb = vec![0; (config.width() * config.height() * 3) as usize];
            CanvasResizer::default().fill_rgb(
                &frame,
                &mut rgb,
                (config.width(), config.height()),
            )?;
            for quality in [35, 75, 90] {
                let mut serial = Vec::new();
                encode_rgb(config, quality, &rgb, &mut serial)?;
                let mut expected = Vec::new();
                Decoder::default().decode_into(config, &serial, &mut expected)?;
                for count in [2, 4] {
                    let mut workspace = JpegWorkspace::with_workers(count);
                    let mut data = Vec::new();
                    workspace.encode(config, quality, &rgb, &mut data)?;
                    let mut actual = Vec::new();
                    Decoder::default().decode_into(config, &data, &mut actual)?;
                    assert_eq!(actual.len(), expected.len());
                    assert!(
                        actual.iter().zip(&expected).all(|(a, b)| a == b),
                        "Restart JPEG pixels differ at {height}p, quality {quality}, {count} workers"
                    );
                    let capacities = workspace.strips.each_ref().map(Vec::capacity);
                    workspace.encode(config, quality, &rgb, &mut data)?;
                    assert_eq!(workspace.strips.each_ref().map(Vec::capacity), capacities);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn oversized_combined_frame_is_atomic() -> Result<()> {
        let config = VideoConfig::new(1080, 60, 90, 30).unwrap();
        let mut rgb = vec![0; (config.width() * config.height() * 3) as usize];
        let mut seed = 0x1234_5678u32;
        for byte in &mut rgb {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *byte = (seed >> 24) as u8;
        }
        let mut output = vec![123];
        let mut workspace = JpegWorkspace::with_workers(4);
        assert!(workspace.encode(config, 90, &rgb, &mut output).is_err());
        assert!(output.is_empty());
        assert!(
            workspace
                .strips
                .iter()
                .all(|buf| buf.capacity() <= MAX_FRAME_BYTES)
        );
        Ok(())
    }

    #[test]
    fn generated_header_rejects_invalid_and_mismatched_strips() -> Result<()> {
        let mut bytes = Vec::new();
        encode_dimensions(16, 8, 75, &[31; 16 * 8 * 3], &mut bytes)?;
        let first = header(&bytes, 16, 8)?;
        assert!(header(&bytes, 16, 16).is_err());
        let mut invalid = bytes.clone();
        invalid[first.height] ^= 1;
        assert!(header(&invalid, 16, 8).is_err());
        assert!(header(&bytes[..bytes.len() - 1], 16, 8).is_err());
        let mut other = Vec::new();
        encode_dimensions(16, 8, 90, &[31; 16 * 8 * 3], &mut other)?;
        let next = header(&other, 16, 8)?;
        assert!(!compatible(&bytes, &first, &other, &next));
        Ok(())
    }
}
