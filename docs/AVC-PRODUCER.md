# Bounded AVC production, recovery and exact modes

Status: local Rust producer, exact operating-point policy, authenticated v9 diagnostic loopback and independent CPU fixture verification. The strict v9 ALPN stays loopback-only; ordinary desktop Compact/Tiles/JPEG and Android local capture authorization are unchanged. This is not a remotely enabled H.264 decoder.

## Producer contract

`h264::producer::Producer` owns one approved encoder epoch and the existing Packetizer. One scheduler owns all mutations. Call push for a complete local access unit, with explicitly known framing, key flag, actual encoder PTS in microseconds and exact-size Box<[u8]>; call notify_loss for upstream fragment/loss; call poll only on a real transport demand. Use the existing recycled output allocation and transport credit. The queue retains at most two pending pictures (2 * MAX_ACCESS_UNIT, under 4 MiB), not counting the caller's current input, output or existing transport window. No intermediate full-size packetizer scratch frame.

The local arrival age limit is 150 ms. Overflow, expired pending picture, upstream loss or strict GOP expiry invalidate all unsent dependent references. Discard until a fresh IDR; request a keyframe at most once per 250 ms during recovery and fail closed after 2 s without emitting a valid IDR. A received IDR is still checked by the existing strict packetizer; changed CSD, unsupported profile, malformed picture, wrong key flag, non-increasing/negative PTS, clock regression or canceled epoch cannot publish data. Existing immutable parameter sets and ordered sequence remain in force. Sequence is assigned only at demand; no synthetic Repeat or dropped wire sequence. The consumer must close the session if an issued record cannot be sent. No generic picture loss recovery by skipping arbitrary reference frames. These are local queue bounds, not total process memory or a latency guarantee.

## Exact operating points

`h264::inspect_sps` reads a bounded explicit Annex B/4-byte length SPS prefix (one SPS, optional PPS), using the existing safe Rust RBSP reader. Observes Baseline/Main/High, progressive 8-bit 4:2:0, POC type 0/2, constraints, level, visible width/height and reference count. Supports both orientations within a 1920x1080 envelope. It rejects unknown/truncated prefixes, custom High-profile scaling lists, interlace and unsupported POC; no full VUI, slice, CABAC, DPB or arbitrary AVC validation. The optional PPS is identified, not syntax-validated by this observation. No color metadata is invented.

`h264::modes::Mode` is an exact local operating point including SPS metadata, explicitly verified color mode, FPS and bitrate. `select_exact` accepts at most 16 entries each, rejects duplicates and independently applies producer/receiver geometry, rate and bitrate ceilings. It preserves the approved producer preference order, does not rotate or synthesize a format, and requires exact mode equality. Mode construction includes coarse macroblock/rate/reference/bitrate checks; complete coded geometry, DPB, color, decoding and actual capacity must be separately verified. No wire exchange or authority is created. Do not advertise a Mode solely from an SPS, device name or reported codec capability. This local policy is not integrated into an Android network session.

## Qualification

112 core tests passed after adding the producer; 115 after exact modes and SPS observation. Strict core and transport lint passed. The authenticated QUIC test sends original generated NVENC access units 0,3,4,5, with one NoUpdate and exact 1,2,3,4 coded sequence, after invalidating the unsent original units 1,2. No native decoder receives remote payloads.

`python -B tools/verify_avc_recovery_cpu.py` launches a developer-only Rust example that accepts no input and emits the embedded known generated fixture through the producer. External FFmpeg CPU h264 decoder (hwaccel none, two configured threads) decodes the original six and recovered four frames. Original/recovered color metadata checked, exact decoded bytes matched for all retained frames (12,441,600 bytes), sampled YUV max absolute error 1. Original 12,126 bytes, recovered 6,909 bytes. Report benchmarks/avc-cpu-recovery-20261127.json. No GPU, no arbitrary bitstream, no persistent video or network. Development check only; not an application decoder, sandbox, throughput or physical presentation result.

Next: Android actual SPS/PTS observations, verify safe bounded runtime CPU decoder isolation, agree a separately versioned real media capability using authenticated identities and independent user consent. Do not expand v9 or its existing strict parameter parser to silently accept arbitrary vendor bitstreams.

## Preview.9 agreement and native qualification

The exact-mode policy now has a canonical pinned-identity loopback agreement in AVC-NEGOTIATION.md. Android local codec sync-frame request/observation and fixture-only software decoder qualification are separate from the producer. The actual Android encoder's Baseline portrait output is still not admitted to strict v9, and the queue is not wired into an Android media network session. No remote decoder grant or new application media listener.
