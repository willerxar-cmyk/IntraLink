# IntraLink Tiles — v0.9

Status: implemented source, not compiled, run or benchmarked. Runtime codec is safe Rust. No FFmpeg process, new crate dependency or external service. The optional historical Python/FFmpeg development benchmark is unrelated to the runtime.

## Mode negotiation

`--codec compact` (default, mode 2) allows independent lossless full-grid keyframes, JPEG keyframes and basic/compact RGB deltas. `--codec tiles` (mode 1) allows only raw/solid tiles and JPEG. `--codec jpeg` (mode 0) permits only complete JPEG frames. Both peers must explicitly match the approved mode on `intralink/view/7`; older ALPNs fail closed. Diagnostic and vault formats do not change.

Changed-record payload tag 0 means complete JPEG, tag 1 means tile update. Tag 1 requires a hybrid mode and an existing keyframe. Tag 2 is an independent lossless keyframe available only in Compact mode. A repeat has no payload. The 2 MiB maximum changed-record and 512 KiB maximum tile-update limits remain. No encoded intermediate tile updates may be dropped.

## Tile wire format

A fixed row-major 32x32 grid, with actual remaining width/height at edges. A record begins with `u8` tag 1 (delta) or 2 (lossless keyframe) and a nonzero `u16` BE tile count. Each entry begins with `u16` BE row-major index and `u8` representation. Indices are strictly increasing and unique. Tag 2 requires the count to equal the entire grid size, which together with uniqueness and bounds guarantees full coverage. All payloads are absolute RGB values; let N be actual tile pixel count.

| Representation | Code | Payload after entry header |
|---|---:|---|
| Raw | 0 | 3N RGB bytes in row-major order |
| Solid | 1 | 3 RGB bytes |
| Palette | 2 | u8 color count (2–16), 3 bytes per palette color, packed indices (MSB first): 1 bit for 2 colors, 2 bits for 3–4, 4 bits for 5–16. Unused trailing bits must be zero. |
| Runs | 3 | u16 BE run count; each run contains u8 length minus one (1–256 pixels) and 3 RGB bytes. Runs can cross rows. The sum of run lengths must equal N. |

Codes 2 and 3 are valid only in compact mode. Unknown encodings, out-of-range indices, truncated records, invalid padding, inconsistent runs or trailing bytes are rejected. The first pass validates the entire record without heap allocation or canvas mutation. The second pass applies it using bounded stack scratch for one tile. At 1080p there are 2040 tiles, so expansion is limited by the validated output grid, not an arbitrary decompression size. No partial update is committed on malformed input. Fuzzing and security review remain necessary.

The encoder detects exact changed tiles and selects the smallest supported representation (ties favor the earlier candidate). It stops palette discovery above 16 colors and run counting once runs cannot beat the best known candidate. Stack scratch is bounded at one tile; the output buffer is reused. The encoder never produces a compact tile larger than raw RGB, and aborts the whole delta if its budget is exceeded. The delta budget clamps the previous tagged keyframe size to 32–512 KiB. On a needed keyframe, Compact attempts a full-grid exact base under 128 KiB. If it does not fit, the explicitly approved hybrid mode uses JPEG; the old mode is not silently changed. JPEG and basic Tiles modes still use JPEG bases. All modes retain the 2 MiB total payload limit; tile records have the stricter 512 KiB bound. Refresh follows 120 changed delta records, not repeated static frames. From an exact base, subsequent absolute deltas preserve every scaled RGB pixel until the next JPEG base. A JPEG base remains lossy, and unchanged pixels retain its artifacts until refreshed. This is not a globally lossless codec. The size heuristic favors fidelity for UI content and does not guarantee a smaller payload than JPEG.

## Calculated single full-tile sizes

These include the 3-byte record header and 3-byte entry header, but not network framing, ACK, TLS/QUIC/IP or retransmissions. They are arithmetic examples, not measured codec or session results.

| Scene | Basic | Compact |
|---|---:|---:|
| Uniform | 9 | 9 |
| Alternating two colors | 3078 | 141 |
| Two 512-pixel runs, split into four 256-pixel runs | 3078 | 24 |
| 16 cycling colors | 3078 | 567 |
| High-entropy RGB | 3078 | up to 3078 |

## Validation and benchmark

Run direct dependency-free tests (requires stable Rust, not a Cargo registry):

```sh
rustc --edition=2024 --test -D warnings crates/intralink-core/src/media_harness.rs -o intralink-media-tests.exe
./intralink-media-tests.exe
```

Run the full workspace/desktop checks from [VALIDATION.md](VALIDATION.md). Run the comparison:

```sh
cargo run --release -p intralink-core --example tile_benchmark
```

The benchmark generates synthetic static, solid, text-like, stripe, photo-like and full-motion scenes at all three supported resolutions. It compares basic and compact modes at the same 512 KiB standalone budget, warms up buffers, reports exact byte counts and mean encoding/decoding timings, and verifies decoded pixels for each emitted delta. It excludes capture, network, JPEG timing and actual desktop quality. Neither this benchmark nor the Rust tests have run in the authoring environment.

## Remaining work

Native H.264/AV1/VP9 hardware adapters and codec capability negotiation, capture/decode workers, native OS permission flows, network tests, keyframe recovery and 60 FPS validation remain pending. Compact tiles are CPU-only, not hardware acceleration. See [CODECS.md](CODECS.md), [PERFORMANCE.md](PERFORMANCE.md) and [ROADMAP.md](ROADMAP.md).

## Full-grid uniform base sizes (calculated)

| Output | Tiles | Payload bytes |
|---|---:|---:|
| 960×540 | 510 | 3063 |
| 1280×720 | 920 | 5523 |
| 1920×1080 | 2040 | 12243 |

A uniform base uses 3 header bytes plus 6 bytes per tile, including partial edge tiles. These are layout calculations before transport overhead, not measured network results or typical desktop sizes. Run `cargo run --release -p intralink-core --example keyframe_benchmark` to measure deterministic exact keyframes and budget exhaustion.
