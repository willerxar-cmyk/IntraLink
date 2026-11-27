# Codec feasibility and next media backend — v0.9 (historical FFmpeg measurements from v0.6)

## Development-only tool

`tools/codec_benchmark.py` uses Python's standard library to orchestrate a locally installed FFmpeg and FFprobe. It does not participate in the Rust application, require a network, access the screen, install drivers, or transmit information. It writes temporary synthetic files and deletes them when finished. Only an explicitly requested JSON report persists.

```sh
python3 tools/codec_benchmark.py --height 1080 --fps 30 --seconds 2 --threads 2 --output codec-results.json
python3 tools/codec_benchmark.py --height 540 --fps 15 --seconds 1 --codecs h264-nvenc h264-qsv h264-amf --output hardware-results.json
```

The hardware command intentionally tries the selected encoders and reports errors rather than selecting a software fallback. Presence in FFmpeg's list of encoders is not evidence that it can open a hardware session. NVIDIA, Intel and AMD options depend on the installed FFmpeg build, driver, device and pixel format. A VAAPI path with explicit device selection remains a future addition. The tool reports failures with a nonzero exit code, but still writes a report. A successful encode/decode does not certify actual GPU utilization or hardware decoding; verify utilization and device selection separately on the target host.

## Executed CPU experiment

See [raw CPU report](../benchmarks/codec-local.json) and [hardware attempts](../benchmarks/hardware-local.json). The CPU run used FFmpeg 7.1.5, synthetic testsrc2 motion, a losslessly compressed 1920x1080 source, 30 FPS, 60 frames, 2 configured threads. All four files passed decoding and frame-count/dimension checks. Process wall time includes reading the source, initialization, conversion, compression, container writing and exit; decode wall time includes reading and decoding to null. Encoded size includes the Matroska container. One run, no quality matching or native screen content. The two-thread setting does not guarantee exactly two OS threads.

| Profile | Encode wall FPS | Decode wall FPS | Encoded container Mbps at 30 FPS |
|---|---:|---:|---:|
| MJPEG | 156.65 | 228.70 | 18.6183 |
| H.264 | 158.17 | 353.69 | 15.6597 |
| VP9 | 67.92 | 212.60 | 8.7150 |
| AV1 | 44.45 | 225.95 | 6.0304 |

These are not comparable quality scores, network rates, native capture FPS or a latency benchmark. No real GPU was available. NVENC failed loading the CUDA runtime, QSV failed creating a Media SDK session, and AMF was absent from the installed FFmpeg build. These failures characterize this runtime only, not the codecs on another machine.

## Integrated Rust tile/JPEG preview (v0.9, source only)

The core now implements an original absolute RGB tile-update format and the desktop CLI negotiates compact tiles, basic tiles or independent JPEG mode on ALPN v6. The receiver no longer drops encoded intermediate records. See [TILES.md](TILES.md). Compact mode also supports a complete lossless RGB base within the sender policy budget. This is not integrated H.264, AV1, VP9 or GPU acceleration.

## Remaining integrated video engine work (not implemented)

1. Complete Rust compilation and tests and exercise the v6 credit window at controlled RTT, loss and bandwidth. Establish real capture-to-presentation baseline.
2. Define a separate versioned media protocol and negotiated decoder capability. Keep dimensions, packet size, sequence, keyframe state, decoder memory, reference-frame count and session duration bounded. A new codec must never be selected solely because the encoder name exists.
3. Use H.264 as the first candidate for low-latency hardware paths; evaluate platform APIs (Media Foundation, VideoToolbox, VAAPI, hardware vendor APIs), licensing, and actual supported profiles. Choose the codec only with explicit verified support at both ends. The present JPEG mode remains an explicit preview, not a silent fallback.
4. For stateful codecs, preserve reference dependencies: the old v0.6 latest-value JPEG mailbox could not preserve arbitrary H.264/VP9/AV1 packets. v0.9 now uses a bounded ordered queue for tile dependencies; add codec-specific keyframe recovery, decoder reset and presentation policy before integrating inter-frame compression. Separate codec configuration from media data; reject unsupported formats and oversized dimensions before allocation.
5. Compare quality-matched UI/text, static desktop, scrolling and motion on real hardware. Record p50/p95 glass-to-glass latency, CPU/GPU use, RSS, achieved FPS, loss behavior and actual network bytes. Do not claim 1080p60 or GPU acceleration before evidence.

## Native NVIDIA diagnostics (v0.22.0 development)

The optional Windows `nvenc-preview` feature now exercises real same-adapter GPU NV12-to-H.264 NVENC and native CUDA/CUVID H.264-to-NV12 decode on generated color-band frames. The diagnostic requires the SDK 13.1 driver ABI, explicit NVIDIA DXGI adapter selection, bounded packets and exact profile, and has no runtime FFmpeg dependency. It checks frame/timestamp continuity, IDR and SPS/PPS presence, native decode status and sampled color values. Independent FFmpeg/ffprobe verification is available via `tools/native_codec_verify.py`; it is a development check, not a runtime dependency.

Debug 1080p native round trip: 180/180 frames, 4 IDR, 158,564 bytes of compressed payload, sampled YUV maximum error 1; conversion + encode p50 3.001 ms, p95 3.524 ms, and native decode map/sample p50 1.946 ms. These timings exclude real capture, network, presentation and the generated-frame upload. The decoder timing excludes parser/submit; do not add stage medians to infer glass-to-glass latency. The observed opaque encoder hardware-status field is 2; its interpretation remains a production-review item. Both API return codes and actual decoded content were checked. See [HARDWARE.md](HARDWARE.md), [PERFORMANCE.md](PERFORMANCE.md) and [APP-VALIDATION.md](APP-VALIDATION.md).

Current v8 continues to negotiate only the existing explicit tile/JPEG modes. Do not advertise H.264 before a bounded versioned access-unit protocol, hostile-bitstream tests, same-device capture/interop, resynchronization and an ordered presentation path are implemented and validated. The previous WGC fixture used Intel; selecting NVIDIA for native diagnostics does not prove that physical capture will be on the same device. The Intel QSV GPU texture-input path is not qualified.

## H.264 v9 loopback diagnostic

The optional NVIDIA fixture now exposes a bounded collection of generated access units and the exact originating D3D11 device. A new `desktop-h264-check` command uses authenticated Iroh QUIC loopback, an exact agreed v9 H.264 profile and per-endpoint structural guards. It validates byte identity and order before submitting received samples to native NVDEC. Encoding is performed before transfer and decoding after transfer; timings are not live capture-to-display latency or a sustained physical-network benchmark. See [PROTOCOL.md](PROTOCOL.md). Existing production-preview Host and viewer commands remain v8-only.


### Restricted headless fixture gate

The existing NVDEC parser and native decoder can execute under the same reduced token without instantiating a graphics presenter. A developer-only example embeds six exact known generated 1080p NVENC packets; no arbitrary data input. The Debug diagnostic validated real immediate decode callbacks, order, sampled colors and teardown (6/6, exit 0). The v9 remote-content boundary remains unchanged. Cross-process surfaces, full hostile input containment and physical session qualification are still required.


## Portable software baseline (2026-11-26)

Compact/Tiles/JPEG are the required native desktop preview baseline and do not use CUDA. `desktop-preview` independently builds the CPU codec path; `wgc-preview` can add D3D11 capture/scaling and `nvenc-preview` adds NVIDIA-specific diagnostics. Do not interpret NVIDIA hardware qualification as a universal decoder. Future H.264 hardware negotiation must verify actual encode/decode capability on both sides, not the GPU vendor alone. The optional headless NVIDIA worker remains limited to its trusted fixture; other hardware and software H.264 implementations are not qualified. Keep the software v8 path available and explicit, not a hidden downgrade from a selected hardware or security mode. Adaptive JPEG quality is a local bounded policy; it does not alter lossless tiles or enable a new codec.

## Local AVC packetizer (2026-11-27)

Shared Rust adapter implements complete Annex B/length-prefixed units, separate SPS/PPS and missing AUD normalization, directly to existing v9 envelopes. Host transport tests and exact native generated 1080p AVC NAL comparisons passed. Not general Android codec compatibility or a remotely reachable AVC decoder. See [AVC-PACKETIZER.md](AVC-PACKETIZER.md).
