//! Same-adapter WGC -> NV12 -> NVENC -> NVDEC on the generated test window.
//! No monitor selection, network listener or real desktop capture.
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use windows::{Win32::Graphics::Dxgi::IDXGIDevice, core::Interface};

use crate::{
    GpuNv12,
    adapter::create_video_device,
    nvdec::{Decoder, FixturePattern},
    nvenc::NativeEncoder,
    nvenc_check::{FIXTURE_BITRATE, FIXTURE_FPS},
    selected_wgc::{Apartment, SelectedCapture},
};

/// A native-generated packet, constructible only inside the diagnostic
/// encoder. The network diagnostic compares received bytes with this token
/// before handing it to the native decoder. Never construct from wire input.
pub struct TrustedWgcPacket {
    sequence: u32,
    keyframe: bool,
    payload: Vec<u8>,
    started: Instant,
}

impl TrustedWgcPacket {
    pub fn sequence(&self) -> u32 {
        self.sequence
    }
    pub fn keyframe(&self) -> bool {
        self.keyframe
    }
    pub fn bytes(&self) -> &[u8] {
        &self.payload
    }
}

enum Sink<'a> {
    Local(bool),
    Stream {
        demand: &'a mut dyn FnMut() -> Result<bool>,
        deliver: &'a mut dyn FnMut(TrustedWgcPacket) -> Result<bool>,
    },
}

pub fn verify_wgc_codec(
    pid: u32,
    title: &str,
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    present: bool,
) -> Result<()> {
    run_fixture(
        pid,
        title,
        height,
        samples,
        adapter_index,
        Sink::Local(present),
    )
    .map(|_| ())
}

/// Synchronous source worker. Capture remains limited to the spawned test
/// window; all native state is owned by the thread running this function.
pub fn stream_wgc_fixture(
    pid: u32,
    title: &str,
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    mut demand: impl FnMut() -> Result<bool>,
    mut deliver: impl FnMut(TrustedWgcPacket) -> Result<bool>,
) -> Result<usize> {
    run_fixture(
        pid,
        title,
        height,
        samples,
        adapter_index,
        Sink::Stream {
            demand: &mut demand,
            deliver: &mut deliver,
        },
    )
}

fn run_fixture(
    pid: u32,
    title: &str,
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    mut sink: Sink<'_>,
) -> Result<usize> {
    let present = matches!(sink, Sink::Local(true));
    let streamed = matches!(sink, Sink::Stream { .. });
    ensure!(
        matches!(height, 540 | 720 | 1080) && (1..=180).contains(&samples),
        "Unsupported WGC codec diagnostic profile"
    );
    let size = (u32::from(height) * 16 / 9, u32::from(height));
    let apartment = Apartment::new()?;
    let (device, context) = create_video_device("nvidia", adapter_index)?;
    let dxgi: IDXGIDevice = device.cast()?;
    // SAFETY: live selected adapter descriptor, no device mutation.
    let adapter = unsafe { dxgi.GetAdapter()?.GetDesc()? };
    let name = String::from_utf16_lossy(&adapter.Description);
    println!(
        "selected_capture_adapter={}, luid={:08x}:{:08x}",
        name.trim_end_matches('\0'),
        adapter.AdapterLuid.HighPart as u32,
        adapter.AdapterLuid.LowPart
    );
    let mut capture = SelectedCapture::start(&apartment, pid, title, &device, &context)?;
    let source = capture.size();
    ensure!(
        source.0 >= size.0 && source.1 >= size.1,
        "Selected WGC source smaller than output"
    );
    let expected = u64::from(size.0) * u64::from(source.1);
    let actual = u64::from(source.0) * u64::from(size.1);
    ensure!(
        actual.abs_diff(expected) <= expected / 100,
        "Unexpected WGC fixture aspect ratio"
    );
    let mut converter = None::<GpuNv12>;
    let mut encoder = None::<NativeEncoder>;
    let mut decoder = if streamed {
        None
    } else {
        let mut decoder = Decoder::with_pattern(&device, size, FixturePattern::Quadrants)?;
        if present {
            decoder.enable_preview(&device)?;
        }
        Some(decoder)
    };
    let mut packet = Vec::new();
    let mut bytes = 0usize;
    let mut keys = 0usize;
    let mut capture_times = Vec::with_capacity(usize::from(samples));
    let mut encode_times = Vec::with_capacity(usize::from(samples));
    let mut decode_times = Vec::with_capacity(usize::from(samples));
    let started = Instant::now();
    let mut delivered = 0usize;
    let operation = (|| {
        for i in 0..samples {
            ensure!(
                started.elapsed() < Duration::from_secs(30),
                "WGC codec fixture deadline exceeded"
            );
            if let Sink::Stream { demand, .. } = &mut sink
                && !demand()?
            {
                break;
            }
            let frame_started = Instant::now();
            let before = frame_started;
            let surface = capture.next(Duration::from_secs(2))?;
            capture_times.push(before.elapsed());
            let before = Instant::now();
            if converter.is_none() {
                converter = Some(GpuNv12::new(surface, size)?);
            }
            let converted = converter
                .as_mut()
                .context("Missing NV12 converter")?
                .convert(surface)?;
            ensure!(converted.size() == size, "Unexpected NV12 geometry");
            if encoder.is_none() {
                encoder = Some(NativeEncoder::new(
                    &device,
                    converted.texture(),
                    FIXTURE_FPS,
                    FIXTURE_BITRATE,
                )?);
            }
            let picture = encoder.as_mut().context("Missing NVENC session")?.encode(
                converted.texture(),
                &mut packet,
                i == 0,
            )?;
            encode_times.push(before.elapsed());
            keys += usize::from(picture.keyframe);
            bytes = bytes
                .checked_add(packet.len())
                .context("Encoded bytes overflow")?;
            let before = Instant::now();
            let accepted = match &mut sink {
                Sink::Local(_) => {
                    decoder
                        .as_mut()
                        .context("Missing local decoder")?
                        .submit(&packet)?;
                    true
                }
                Sink::Stream { deliver, .. } => deliver(TrustedWgcPacket {
                    sequence: u32::from(i) + 1,
                    keyframe: picture.keyframe,
                    payload: packet.clone(),
                    started: frame_started,
                })?,
            };
            decode_times.push(before.elapsed());
            if !accepted {
                break;
            }
            delivered += 1;
        }
        Ok::<(), anyhow::Error>(())
    })();
    let elapsed = started.elapsed();
    // Stop capture before codec teardown on success, cancellation or error.
    let capture_cleanup = capture.close();
    let closed_rejected = capture.next(Duration::from_millis(1)).is_err();
    let encoder_cleanup = match encoder.take() {
        Some(encoder) if operation.is_ok() => encoder.finish(),
        Some(encoder) => encoder.abort(),
        None => Ok(()),
    };
    let decoder_cleanup = match decoder {
        Some(decoder) if operation.is_ok() && delivered == usize::from(samples) => decoder.finish(),
        Some(decoder) => decoder.abort(),
        None => Ok(()),
    };
    drop(converter);
    // SAFETY: all native work has ended and the immediate context is owned by
    // this diagnostic; release bound graphics state before device teardown.
    unsafe {
        context.ClearState();
        context.Flush();
    }
    drop(capture);
    println!("selected_wgc_resources_released");
    drop(dxgi);
    drop(context);
    drop(device);
    println!("selected_d3d11_device_released");
    drop(apartment);
    println!("selected_winrt_apartment_released");
    capture_cleanup?;
    encoder_cleanup?;
    decoder_cleanup?;
    operation?;
    ensure!(closed_rejected, "Closed capture accepted another frame");
    if streamed && delivered != usize::from(samples) {
        println!(
            "wgc_stream_source=cancelled, delivered={delivered}, encoded={}, resources_released=true",
            encode_times.len()
        );
        return Ok(delivered);
    }
    ensure!(
        delivered == usize::from(samples),
        "Incomplete WGC diagnostic"
    );
    let rank = |times: &mut Vec<Duration>| {
        times.sort_unstable();
        (
            times[(times.len() * 50).div_ceil(100) - 1].as_secs_f64() * 1000.0,
            times[(times.len() * 95).div_ceil(100) - 1].as_secs_f64() * 1000.0,
        )
    };
    let (cap50, cap95) = rank(&mut capture_times);
    let (enc50, enc95) = rank(&mut encode_times);
    let (dec50, dec95) = rank(&mut decode_times);
    if streamed {
        println!(
            "wgc_stream_source=passed, frames={samples}, output={}x{}, bytes={bytes}, idr={keys}, elapsed_ms={:.3}, source_fps={:.2}, capture_p50_ms={cap50:.3}, encode_p50_ms={enc50:.3}, enqueue_p50_ms={dec50:.3}, encoder_pixel_readback=0",
            size.0,
            size.1,
            elapsed.as_secs_f64() * 1000.0,
            f64::from(samples) / elapsed.as_secs_f64()
        );
        return Ok(delivered);
    }
    println!(
        "wgc_codec_check=passed, native_source={}x{}, output={}x{}, frames={samples}, idr={keys}, payload_bytes={bytes}, elapsed_ms={:.3}, local_fps={:.2}, capture_p50_ms={cap50:.3}, capture_p95_ms={cap95:.3}, convert_encode_p50_ms={enc50:.3}, convert_encode_p95_ms={enc95:.3}, decode_submit_p50_ms={dec50:.3}, decode_submit_p95_ms={dec95:.3}, encoder_pixel_readback=0, explicit_nvidia_device=true, same_device=true, closed_capture_rejected=true, gpu_preview={present}",
        source.0,
        source.1,
        size.0,
        size.1,
        elapsed.as_secs_f64() * 1000.0,
        f64::from(samples) / elapsed.as_secs_f64()
    );
    println!(
        "Owned moving test window, native hardware encode/decode and sampled color verification; optional local GPU presentation; not physical-display sharing or a network session."
    );
    Ok(delivered)
}

/// Native decoder/presenter worker receives only private native-generated
/// tokens. The caller must compare each token to its received wire envelope.
/// Callback runs after successful native submission; it is not a scan-out ACK.
pub fn verify_wgc_stream<F>(
    height: u16,
    samples: u16,
    adapter_index: Option<u32>,
    mut next: impl FnMut() -> Result<Option<(TrustedWgcPacket, F)>>,
) -> Result<usize>
where
    F: FnOnce() -> Result<bool>,
{
    ensure!(
        matches!(height, 540 | 720 | 1080) && (1..=180).contains(&samples),
        "Invalid diagnostic profile"
    );
    let size = (u32::from(height) * 16 / 9, u32::from(height));
    // The apartment and device are created and destroyed on this worker,
    // independently of the selected WGC encoder device.
    let apartment = Apartment::new()?;
    let (device, context) = create_video_device("nvidia", adapter_index)?;
    let mut decoder = Decoder::with_pattern(&device, size, FixturePattern::Quadrants)?;
    decoder.enable_preview(&device)?;
    let mut timings = Vec::with_capacity(usize::from(samples));
    let mut first_submit = None::<Instant>;
    let mut last_submit = None::<Instant>;
    let operation = (|| {
        for sequence in 1..=u32::from(samples) {
            let Some((packet, accepted)) = next()? else {
                break;
            };
            ensure!(
                packet.sequence == sequence,
                "Native packet sequence mismatch"
            );
            decoder.submit(&packet.payload)?;
            let submitted_at = Instant::now();
            first_submit.get_or_insert(submitted_at);
            last_submit = Some(submitted_at);
            timings.push(submitted_at.duration_since(packet.started));
            if !accepted()? {
                break;
            }
        }
        Ok::<(), anyhow::Error>(())
    })();
    let submitted = timings.len();
    let cleanup = if operation.is_ok() && submitted == usize::from(samples) {
        decoder.finish()
    } else {
        decoder.abort()
    };
    // SAFETY: the decoder and its presenter are closed, and this context
    // belongs only to the diagnostic viewer worker.
    unsafe {
        context.ClearState();
        context.Flush();
    }
    drop(context);
    drop(device);
    drop(apartment);
    cleanup?;
    operation?;
    if submitted != usize::from(samples) {
        println!(
            "wgc_stream_viewer=cancelled, submitted={submitted}, worker_resources_released=true"
        );
        return Ok(submitted);
    }
    let steady_fps = match (first_submit, last_submit) {
        (Some(first), Some(last)) if last > first => {
            (submitted - 1) as f64 / last.duration_since(first).as_secs_f64()
        }
        _ => 0.0,
    };
    timings.sort_unstable();
    let p50 = timings[(timings.len() - 1) / 2].as_secs_f64() * 1000.0;
    let p95 = timings[(timings.len() * 95).div_ceil(100) - 1].as_secs_f64() * 1000.0;
    println!(
        "wgc_stream_viewer=passed, submitted={samples}, demand_to_decode_submit_p50_ms={p50:.3}, p95_ms={p95:.3}, steady_decode_submit_fps={steady_fps:.2}, worker_resources_released=true"
    );
    Ok(submitted)
}
