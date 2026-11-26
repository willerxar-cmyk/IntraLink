# Performance engineering — v0.9

## Implemented design (not yet benchmarked)

The v0.4 desktop preview coupled capture, network and presentation, always encoded JPEG, and allocated an output pixel vector for each frame. The replacement moves QUIC into a Tokio task, requests exactly one capture on demand through a capacity-one channel, recycles JPEG capacity, and retains two reusable output RGB canvases for exact comparison. It sends a repeat record instead of a JPEG when the canvas is unchanged. In v0.9 the viewer receives changed media records in an ordered capacity-one queue and reuses its decoded presentation buffer. No unbounded queue or arbitrary encoded-frame dropping. No new external Rust dependencies are required by this change.

This is not zero-copy. Native capture and resize may allocate, the receiver owns a compressed buffer per changed frame (recycled when available from v0.13 onward), JPEG decoding allocates, and the GUI repaints its current buffer to pump events. Host native capture/encoding and viewer decoding can block their own GUI loops. At 1080p, two RGB canvases require 12,441,600 bytes and the u32 presentation buffer needs 8,294,400 bytes, excluding native images, temporary codec memory, OS, QUIC, and all other app state. These are allocation-size calculations, not RSS measurements. Exact comparison avoids hash collision behavior; it operates on scaled RGB, not the original native screenshot. At 30 repeats/s, the repeat record plus receipt ACK is 240 application bytes/s, or 80 bytes/s at 10 repeats/s, excluding protocol setup and QUIC/IP overhead. This does not represent measured bandwidth. Changed frames now carry a JPEG keyframe, a lossless Compact keyframe or a Rust tile update in explicitly negotiated hybrid mode.

v0.5 used strict ACK-per-record stop-and-wait. v0.6 through v0.14 permit up to three records in flight before consuming the next ordered ACK. A full credit window stalls capture; a five-second aggregate receipt drain precedes normal termination. This bounds the conforming-peer unacknowledged media payload to 6 MiB, plus headers and other allocations. It does not imply a 6 MiB RSS cap or proof of rendering. Under a simplified zero-processing/serialization model, the receipt-limited rate ceiling changes from 1/RTT to 3/RTT, subject to the configured FPS cap; actual throughput may be lower and buffering can add latency. No network benchmark was run. JPEG remains CPU-only, and the window is fixed rather than congestion-adaptive. See [CODECS.md](CODECS.md) for actual local synthetic FFmpeg codec results, separate from the Rust application.

## Existing CLI instrumentation

Run both endpoints with release builds on an isolated owner-controlled network. Use the README setup, match `--codec compact`, `--codec tiles` or `--codec jpeg` on both peers, and select host `--height 540|720|1080`, `--fps 1..30`, `--quality 35..90`, `--seconds 1..60`. The final successful session report prints changed media records, repeat records, media payload bytes and foreground elapsed time (including connection wait/cleanup). Both sides report local JPEG keyframe, lossless keyframe and tile-update counts. Host additionally prints local capture and resize/compare/encode sample count and total durations; viewer prints count of decoded changed frames and total decode duration. These are stage timings, not input-to-display latency, network bitrate or GPU usage. Local cancellation may not have complete transport counters. No screen payload or secrets are intentionally written to a metrics file.

## Reproducible test matrix

Use a real local display with a disposable account and no sensitive windows. Record commit, Cargo.lock hash, OS, CPU/GPU, display size, renderer, release compiler, connection type and network conditions. Test each scenario three times for the same 30-second window after connection, keeping settings fixed. Capture machine-side RSS/CPU using native tools, actual network bytes using an owner-controlled packet capture, and local timing counters. Repeat identical scenarios on v0.5 to isolate the credit-window change, and on v0.4 where settings overlap (540p/3 FPS/quality 75). Record failures rather than omitting them.

| Scene | Profiles | What to record |
|---|---|---|
| Static desktop (no animated elements) | 540p/15; 1080p/15 | Repeat ratio, payload bytes, CPU/RSS, stage durations |
| Normal scrolling / moving windows | 540p/15; 720p/15; 1080p/15 | Changed frame throughput, latency, decode/presentation count |
| High-motion local test video | 720p/30; 1080p/30 | CPU, memory, bandwidth, achieved rate; no recording |
| Controlled slow / lossy network | 540p/15; 720p/30 | ACK delay, timeout, memory plateau, cancel behavior |
| Controller closes during active sharing | All profiles | Listener cleanup, no subsequent capture/transmission |
| Host closes while controller stalls or waits | All profiles | UI stop response and network teardown |

For glass-to-glass latency, use a visible local timestamp or test animation and an external high-frame-rate camera covering both displays; record p50/p95 after startup. Measure cancellation against a monotonic timer, not just log order. Compare user-visible quality on text, small UI elements and motion. No end-to-end performance numbers are supplied for a run that did not happen.

## Release gate

Run `cargo fmt --all`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace --release`, and the optional desktop feature equivalents on each supported native platform. Resolve and commit a real Cargo.lock, fix all diagnostics, and audit dependencies. Verify direct-only traffic and consent. Do not claim a speedup or a supported FPS without comparative data. Next work: native permission-aware capture adapters, bounded independent capture worker, hardware codec interfaces, measured adaptive congestion/credit control, native keystore and revocation. Only then add authorized input and other privileged modules.

## Rust tile microbenchmark (not run)

Run `cargo run --release -p intralink-core --example tile_benchmark` and save its CSV with machine/build metadata. This synthetic core-only benchmark does not measure full JPEG encoding, GUI capture, network or GPU. Compare tiles and JPEG end-to-end with equal resolution, frame limits and quality settings; record CPU/RSS, latency, motion quality, payload bytes and keyframe/tile mix. See [TILES.md](TILES.md).

## v0.9 exact base measurements (not executed)

The Rust `keyframe_benchmark` example reports keyframe payload bytes, mean encode/decode time, exact pixel verification and refusal for the 128 KiB sender policy. It uses only deterministic synthetic scenes; no live capture or GPU. Native full-size capture now skips clearing the RGB scratch canvas before overwriting every pixel. Measure the benefit; no speedup is claimed.

```sh
cargo run --release -p intralink-core --example keyframe_benchmark
```

## Adaptive static-screen pacing and receive recycling (v0.13)

In v0.13–v0.14 the host observes successful record types only; v0.15 additionally accepts bounded priority feedback after ordered receipt validation. Following 4–11 consecutive repeats, the next start-to-start interval is twice the configured interval; at 12 or more, four times. The interval is capped at 100 ms for profiles faster than 10 FPS and never shorter than the original interval. A changed record restores the base interval. The existing deadline and receipt credit still apply. This is content-adaptive sampling, not network congestion control. For 30 FPS, nominal active / intermediate / static rates are 30 / 15 / 10 FPS before capture and network overhead; a sustained static scene can therefore require two-thirds fewer captures than the original 30 FPS target. An intervening screen change may take up to the next sampling interval to be noticed in the no-stall case; short-lived changes can be missed. No real-device latency or CPU savings have been measured.

A capacity-two nonblocking channel returns compressed payload allocations from the viewer to its network task after decoding. When no allocation is available the reader allocates normally. When the pool is full it drops the returned allocation. Payload length remains at most 2 MiB; no arbitrary frame is dropped, and the changed-record queue remains capacity one. Receive buffer growth uses `try_reserve_exact`; the reader discards a recycled allocation whose capacity exceeds the payload limit. Reuse avoids some allocator calls, not zero initialization, TLS/OS copies, native capture, JPEG decoding, or repaint. This is not an RSS bound.

Benchmark v0.12 versus v0.13 with the same build profile, connection, screen and 30 FPS setting. Include static screen, short interaction bursts after idle, continuously moving image and slow consumer. Record capture count, repeat count, memory allocation/RSS, CPU, and p50/p95 visual latency. Check that no changed record is lost or reordered and local cancellation remains functional. Tests of the pure pacing policy and a loopback recycle regression are defined, not executed in the authoring environment.

## v0.15 multi-device scheduling

The protocol cap is 60 FPS (16.67 ms nominal start-to-start) and the fixed credit window is eight records. The host polls its consent window independently instead of sleeping twice for every frame. The workspace uses separate decode workers and only presents the foreground canvas. A foreground profile of 60 and two active background profiles of 5 imply at most 70 nominal captures/s versus 180 in all-live mode, assuming all hosts authorize 60 FPS and screen content changes. Static-scene pacing and stalls reduce these ceilings further. The numbers are policy arithmetic, not network/CPU measurements. See [WORKSPACE.md](WORKSPACE.md) for the native acceptance gates.

## v0.16 immutable presentation snapshots

The workspace presenter now clones a reference to the decoded canvas rather than holding the shared lock throughout the native window update. Decoding can proceed during a native presentation call; the decoder reuses its allocation when unshared and performs safe copy-on-write when a snapshot overlaps. At 1080p a full RGB32 canvas is 8,294,400 bytes (about 7.91 MiB). The single transient presenter can add one extra canvas to the base per-session allocations, excluding codec, transport and native window memory.

Run from the repository root:

```sh
rustc --edition=2024 -D warnings -O tools/presentation_benchmark.rs -o presentation-benchmark.exe
./presentation-benchmark.exe
```

Actual Windows x64 run using rustc 1.96.0: three 1920x1080 surfaces, 240 rounds, three one-pixel updates per round (720 updates in each scenario). Without holding an old snapshot: 0 observed copy-on-write events, 0.013 ms elapsed (reported 0.053 microseconds per round; extremely small synthetic workload). With an old foreground snapshot deliberately held across every update: 240 observed copies, 418.475 ms elapsed, 1743.645 microseconds per round. The latter is about 1.74 ms of a synthetic copy-heavy round. Initialization, real image capture, decoding, compression, GUI, rendering, GPU, networking and a timed FPS schedule are excluded. These are single-run microbenchmark measurements, not comparative end-to-end performance, sustained FPS, or a guarantee of application latency. See [the report](../benchmarks/presentation-windows-validation.json).

Next measure actual per-session capture, decode and presentation rates and p95 latency with multiple native displays, then compare GPU-native codecs and presentation.

## v0.17 overview budget

The additional canvas is exactly 960 × 540 × 4 = 2,073,600 bytes (about 1.98 MiB, excluding allocator overhead), allocated on first opening. It is reused and cleared on closing. Thumbnail rendering reads each existing decoded snapshot locally, one at a time. The overview nominally redraws every 100 ms while visible (and on user actions), independently of the 60 FPS profile. These are design calculations, not measured CPU, presentation throughput, memory RSS or network FPS. Verify the full native workspace under 1/3/8 concurrent sessions and compare panel open/closed with equal media profiles. The six new tests were executed, but no additional performance benchmark ran.

## Windows maintenance: capture demand wakeup

The Host's idle capture-demand wait uses Tokio's channel receive with an 8 ms UI deadline, replacing the 1 ms sleep/try-receive polling loop. A capture request wakes the asynchronous wait without waiting for the timer. The UI thread still pumps native events and checks cancellation before capture. This reduces the nominal idle polling frequency; measured CPU savings, timer jitter, glass-to-glass latency, and sustained frame rates are not established by the change alone. Native capture and encoding may still block their owning thread.

## Native window media baseline — Windows, 2026-11-26

The new `desktop-media-check` command uses XCap 0.9.10's default Windows GDI window backend, the actual application CPU encoder/decoder and MiniFB buffer submission. The source is an animated local diagnostic child window; no real desktop, other application, network stream or GPU codec is measured. The physical capture dimensions differ from the logical source window due to native scaling. The pipeline processes samples serially, at a ceiling of 60 samples/s; latency percentiles are wall-clock observations, not glass-to-glass timings. Host hardware observed: Core i9-12900HK, NVIDIA RTX 3080 Ti Laptop GPU and Intel Iris Xe (3840x2160 display).

| Output | Native capture | Codec | Samples | Observed samples/s | Capture p50 ms | Encode p50 ms | Decode p50 ms | Pipeline p95 ms | Payload bytes |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|
| 960x540 | 1641x924 | Compact | 60 | 11.65 | 60.682 | 24.522 | 0.008 | 101.022 | 184596 |
| 1920x1080 | 3321x1869 | Compact | 30 | 1.81 | 442.277 | 117.487 | 0.018 | 639.621 | 148796 |
| 960x540 | 1641x924 | JPEG | 60 | 10.26 | 63.037 | 34.730 | 1.476 | 116.929 | 1484083 |

All three runs exited 0 and observed changed records for every sample. See [raw stage measurements](../benchmarks/native-window-media-windows-20261126.txt). The workload changes continuously but its motion speed is not fixed between runs; these figures are not a controlled codec A/B comparison. Capture and resize/encoding dominate this local diagnostic. No physical LAN throughput, remote display capture FPS or CPU/GPU utilization claim is made. Next benchmark an event-driven native display capture implementation and keep local consent and visible stop checks intact.

## SIMD resize optimization — Windows, 2026-11-26

The shared production CPU encoder now uses fast_image_resize 6.2.0 (MIT OR Apache-2.0), restricted to RGBA8, with antialiased bilinear convolution and automatic CPU extension selection. Screen RGB is treated as opaque regardless of the unused fourth channel. The intermediate scaled RGBA buffer and library workspace are retained per encoder, and matching-size frames skip resizing entirely. The existing RGB canvas, exact comparisons and wire records remain unchanged. The new tests cover aspect/rounding, antialiasing against the earlier triangle filter, RGB preservation when alpha is zero, geometry rejection, and scratch reuse.

| Output / codec | Previous median resize+encode | SIMD median resize+encode | Local samples/s before | Local samples/s after |
|---|---:|---:|---:|---:|
| 540p Compact | 24.522 ms | 2.673 ms | 11.65 | 12.09 |
| 1080p Compact | 117.487 ms | 11.386 ms | 1.81 | 1.85 |
| 540p JPEG | 34.730 ms | 14.547 ms | 10.26 | 9.74 |

The three repeated GDI-window diagnostic runs all passed (150 changed samples); see [raw measurements](../benchmarks/native-window-simd-windows-20261126.txt). Native capture timing varied significantly between runs (540p Compact median increased from 60.682 to 78.904 ms); no controlled end-to-end throughput improvement or sustained 60 FPS claim follows. The CPU stage measurements include resize and content-dependent encoding. The fixture animation is not locked to a fixed motion speed. Separate full-display, network and hardware encoding gates remain open.

## Persistent Windows Graphics Capture diagnostic results

Locked Release build with `wgc-preview,resource-monitor`, three generated-window trials, 60 requested samples each. The WGC backend keeps the capture session open, waits for new frame events and maps the frame into a CPU buffer only on demand. Native capture dimensions vary slightly from GDI because the APIs report different window bounds.

| Output / codec | Native source | Samples/s | Changed FPS | Capture p50 ms | Resize+encode p50 ms | Pipeline p95 ms |
|---|---|---:|---:|---:|---:|---:|
| 540p Compact | 1656x933 | 55.64 | 54.72 | 8.941 | 3.395 | 25.890 |
| 1080p Compact | 3336x1878 | 23.19 | 23.19 | 26.142 | 14.649 | 56.491 |
| 540p JPEG | 1656x933 | 28.69 | 28.69 | 12.451 | 17.284 | 47.309 |

All three trials exited successfully, with 179 changed frames out of 180 samples; one repeated frame was handled normally. See [raw results](../benchmarks/native-window-wgc-windows-20261126.txt). Compared with the optimized GDI window diagnostic, observed Compact sample throughput increased from 12.09 to 55.64 at 540p and 1.85 to 23.19 at 1080p. These are separate short runs, not a controlled sustained network A/B experiment; source dimensions, motion and system load differ. WGC still stages and copies CPU-readable data, and the codec and renderer are CPU-based. The next gate is a persistent locally authorized full-display adapter, bounded lifecycle and device-loss handling, GPU-resident scaling/color conversion, and hardware encoder/decoder negotiation. No 1080p60 or full-display remote streaming certification.

## Reusable D3D11 staging and RGBA buffer results

| Output | Before / after samples per second | Capture p50 before / after | Pipeline p95 before / after |
|---|---:|---:|---:|
| 540p Compact | 56.35 / 56.92 | 9.493 / 10.297 ms | 23.657 / 21.290 ms |
| 1080p Compact | 26.37 / 36.88 | 24.882 / 20.777 ms | 48.120 / 34.836 ms |

Each run requested 180 frames; both optimized runs counted one staging texture and one RGBA buffer allocation/growth. Native source dimensions remained 1656x933 and 3336x1878 respectively. Changed-frame rates were 55.66 and 36.47/s in the optimized runs. These are local generated-window trials, not a controlled sustained physical-network experiment. See [raw results](../benchmarks/native-window-readback-windows-20261126.txt).

## D3D11 GPU scaling before readback — measured local results

Four 180-sample Release trials on the same executable with the Compact codec; persistent WGC, retained staging and RGBA allocations in all cases. GPU mode retained one video processor and performed all requested resize work before CPU readback. Exact generated-window color markers were verified on each sample.

| Output / scaling | Source / readback resolution | Samples/s | Changed/s | Capture p50 ms | Encode p50 ms | Pipeline p95 ms | Readback bytes | Media payload bytes |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 540p SIMD | 1656x933 / 1656x933 | 56.68 | 54.79 | 8.126 | 2.576 | 21.500 | 1112434560 | 306723 |
| 540p GPU | 1656x933 / 958x540 | 56.50 | 55.25 | 11.094 | 0.691 | 20.798 | 372470400 | 495194 |
| 1080p SIMD | 3336x1878 / 3336x1878 | 33.11 | 33.11 | 20.603 | 10.030 | 41.400 | 4510805760 | 353227 |
| 1080p GPU | 3336x1878 / 1918x1080 | 48.14 | 47.07 | 11.925 | 2.777 | 33.425 | 1491436800 | 540874 |

GPU 1080p reduced unpadded RGBA readback bytes by about 66.9% and improved observed samples/s by about 45.4% relative to the paired SIMD run; 540p throughput was approximately unchanged. The GPU path produced larger Compact payloads on this animated fixture, possibly due to driver filtering and differing motion sampling. Do not assume superior text quality or bandwidth without controlled screen-content comparisons. The reported bytes are copied texel data, not measured PCIe traffic. These short local tests exclude actual remote networking, hardware encoding, display scanout timing and sustained performance. See [raw results](../benchmarks/native-window-gpu-scale-windows-20261126.txt).

## Latest GPU mailbox first native run — 2026-11-26

The freshly compiled Release mailbox revision completed one 180-poll strict GPU-scale 1080p Compact moving-window trial: 178 readbacks, 174 changed records, 180 native updates, 2 no-update polls; 54.57 observed native samples/s and 53.34 changed/s. Capture p50/p95/p99 9.342/26.083/30.364 ms; encode p50 2.155 ms; pipeline p95 31.155 ms. Source 3336x1878, readback 1918x1080; 1,474,865,280 unpadded readback bytes, 535,124 encoded payload bytes. One staging texture, RGBA allocation and GPU processor. Both static trials failed their marker assertion, so the idle final-update gate was not approved in this run. See `benchmarks/native-window-mailbox-windows-20261126.txt`. These are not controlled comparative trials, network FPS or end-to-end hardware encoding measurements.

### Final mailbox lifecycle build — 2026-11-26
Release 0.22.0, generated-window Compact motion trial: 180 samples, 178 changed, 53.73 capture FPS, 53.14 changed FPS, 540220 payload bytes, 1491436800 readback bytes; one staging texture, RGBA allocation and GPU processor. Source 3336x1878, readback 1918x1080, output 1920x1080. Capture p50/p95/p99: 10.861/21.806/27.872 ms; encode: 3.159/5.115/5.612 ms; local pipeline: 15.013/25.724/31.025 ms. Static checks passed at 540p and GPU-scaled 1080p with no idle CPU readback bytes. Short local diagnostics do not establish sustained 60 FPS or network latency. Compare equivalent visual content and scaling quality before selecting a production default.

### Native ownership and geometry revision
The short 180-sample generated-window GPU-scale Compact run completed at 56.08 capture FPS and 54.21 changed FPS (174 changed records), with p50/p95/p99 local pipeline 14.093/18.425/34.955 ms. Native geometry rejection observed in 47 ms; no additional readback. One staging allocation, RGBA allocation and processor. This is not a controlled performance comparison or sustained physical network stream.

### v8 local regression run
179 native readbacks / 180 samples, 171 changed, 30.80 capture FPS and 29.42 changed FPS. Local pipeline p50/p95/p99 19.842/35.129/49.214 ms; one staging allocation, one RGBA allocation, one processor. Results are not load-controlled or a network benchmark; the preceding revision measured 56.08 capture FPS under different conditions. Benchmark controlled repeated runs before assigning a regression.

## GPU NV12 diagnostic baseline

1920x1080 NV12 contains 3,110,400 packed bytes vs 8,294,400 RGBA bytes (62.5% fewer). This is an uncompressed surface-size difference, not network bandwidth. Native 1080p Debug GPU conversion + validation readback p50 on the selected RTX 3080 Ti Laptop GPU: 2.407 ms forward, 2.579 ms reverse; Intel Iris Xe index 0: 8.977/6.970 ms. Short generated color-band tests, six samples per adapter; no sustained-FPS inference, hardware encoder, or production readback requirement. The retained GPU output is intended for future native encoding without CPU staging.

### NV12 Release results
Three explicit DXGI entries, 60 changing 1080p color-band frames per entry, all passed. NVIDIA forward/reverse with readback p50 1.941/2.026 ms; Intel index 0 4.783/3.903 ms; Intel index 2 5.043/4.060 ms. No zero-copy encoder performance claim. Local WGC Compact motion regression passed with 54.46 capture FPS, 53.85 changed FPS, 178 native samples, 2 no-update samples. p50/p95 pipeline 15.001/24.478 ms. Not a physical network benchmark.

### Native NVIDIA codec diagnostic

On RTX 3080 Ti Laptop, the 180-frame synthetic 1080p native Debug run measured GPU RGBA-to-NV12 + synchronous NVENC + compressed bitstream retrieval p50/p95/p99 3.001/3.524/4.467 ms. No full CPU pixel readback in encoder. Native NVDEC map and six-row color sampling p50 1.946 ms, 180/180 clean decoded display callbacks. This excludes actual capture, network, renderer, and source generation/upload from the encode time; decoder mapping/sample timing excludes parser/submit time. The simple rotating color bands are not representative desktop bitrate or quality. Measure a complete pipeline under controlled physical conditions before declaring sustained 60 FPS.

## H.264 v9 loopback pacing and Windows timer

The 180-frame 1080p generated H.264 stream (160,004 envelope bytes, exact native packet matching and clean decode) passed authenticated loopback transport but the standard Tokio timer measured only 31.61 wire frames/s, 5693.928 ms total. The optional private high-resolution Windows waitable timer measured 57.16 wire frames/s, 3148.983 ms total, on the same generated sequence (both Debug). Other native stages ran before/after transport. This is one short comparison, not controlled sustained physical-network or capture-to-display performance. Measured relative improvement about 80.8%. No system-wide timer-resolution change, busy-spin or thread priority change: one unnamed high-resolution timer on one bounded request/result worker, joined at session end. The transport fails closed if the wait policy returns early or errors; it still applies negotiated FPS and credit limits. Standard wait remains the default and the normal v8 desktop path. `desktop-h264-check --precise-timer` is an explicit diagnostic option. Windows API reference: https://learn.microsoft.com/windows/win32/api/synchapi/nf-synchapi-createwaitabletimerexw . Measure longer controlled sessions and power/CPU before selecting a production pacing backend.

### v9 Release timing confirmation

Precise timer: 55.77/58.15/58.62 wire FPS at 540p/720p/1080p respectively (60/60/180 pre-encoded native frames). Same 1080p fixture standard timer: 32.12 wire FPS. Both 1080p runs delivered identical 160004 envelope bytes and passed exact fixture and NVDEC validation. Release 1080p precise transfer 3070.854 ms, standard 5603.580 ms. Existing WGC local Compact motion fixture passed at 54.76 capture FPS; these separate measurements cannot be combined into live pipeline latency or FPS. No sustained physical network claim.

## Selected NVIDIA WGC local native presentation

The explicitly selected NVIDIA WGC pool captured 3336x1878 generated-window content and scaled to 1920x1080 NV12, then native NVENC/NVDEC and CUDA-D3D11 shader/present. Two final Debug 180-frame runs completed 55.71 and 55.60 local FPS, all 180 Present submissions and decoded content checks, with actual process exit 0. Presentation (plane uploads + RGB shader + 4-pixel validation + Present) p50 0.776/0.729 ms, p95 1.866/0.996 ms; capture p50 11.454/11.676 ms, conversion+encoding p50 2.248/2.199 ms, decode submission including presentation p50 2.876/2.776 ms. These measurements overlap (do not add medians), include startup, and are not a physical display or network capture-to-photon benchmark. The source fixture uses a simple moving bar; the payload/bitrate is not representative desktop content. No full pixel CPU readback; diagnostic reads sampled YUV rows and four RGB pixels only.

### 2026-11-26 — Selected NVIDIA WGC + native GPU presentation: Release gate complete
- Canonical Release build (14,987,264 bytes) completed with exit 0; rustfmt, all three strict Clippy feature gates and 145 workspace tests passed. Cargo.lock unchanged.
- Actual child-process exit 0 in all seven WGC codec runs, including four DXGI presentation runs (480 total presented frames). 1080p presentation repeated: 180/180 frames at 56.14 and 55.02 local FPS, p50 GPU upload/shader/Present 0.715 and 0.757 ms, max sampled RGB error 3/255. No complete-frame CPU readback by encoder; diagnostic sample readbacks remain. Non-present 1080p: 180/180 at 54.70 FPS.
- Existing authenticated direct QUIC synthetic H.264 diagnostic: 180/180, 58.17 FPS. Static capture zero idle readback; geometry rejection passed. Complete gate exit 0. Earlier NVIDIA driver teardown failures remain recorded; corrected release order validated by actual process exits, not only printed markers.
- Evidence: benchmarks/wgc-selected-build-gates-20261126.txt, wgc-gpu-present-release-20261126.json, wgc-codec-release-20261126.json, wgc-selected-regression-release-20261126.txt.
- Artifacts: dist/intralink-windows-x64-wgc-gpu-preview.exe, dist/intralink-0.22.0-wgc-gpu-locked-source.zip, dist/SHA256SUMS-wgc-gpu.txt. Previous artifacts retained.
- Scope remains generated-window local diagnostics and separately pre-encoded loopback; no physical display sharing, untrusted AVC, network-to-GPU presentation, sustained real-network 1080p60 certification, or new input permissions. Next: complete H.264 parameter validation and decoder isolation, connect authorized physical Host and remote Viewer to validated GPU paths, validate controlled real-network session and cancellation/teardown; Intel/other platforms still pending.

## Continuous WGC-to-QUIC-to-DXGI local diagnostic

In contrast to the pre-encoded v9 check, desktop-wgc-live-check captures the generated moving window on explicit NVIDIA WGC, encodes each demand, transfers the native access unit through actual authenticated QUIC, verifies exact origin in-process, then independently decodes and submits to DXGI. Two final Debug 1080p runs of 180 pictures measured steady decode-submit cadence 58.73 and 58.44 FPS, full transport including initialization/end 49.75 and 49.39 FPS, demand-to-submit p50 6.443/9.484 ms, p95 15.904/19.035 ms. Capture timestamps start at the local demand, not photon capture; decoded-submit time is not scan-out. No physical network or full desktop benchmark, no controlled long-duration/CPU/power measurement. These short trials support further work toward 60 FPS, not certification. Eight bounded origin tokens and one decode job, no complete encoder readback; diagnostic YUV rows and four RGB pixels still sampled.

### Continuous GPU/QUIC Release gate completed (2026-11-26)

PID 31504 finished with exit 0 (170.67 s). Formatting, all three strict Clippy feature combinations and all 149 workspace tests passed (31 app unit, 2 CLI, 74 core, 3 D3D11, 26 transport, 2 multi-host, 11 vault). Locked Release executable: 15,181,312 bytes, local timestamp 19:04:10. Cargo.lock byte-identical to previous validated archive; all modified Rust files remain under 800 lines. The live verifier passed all seven actual process runs. Continuous 540p 60/60, 720p 60/60 and two 1080p 180/180 DXGI presentations: steady decode-submit cadence 60.47, 60.04, 59.16 and 58.31 FPS; overall wire pipeline including startup and end 35.68, 39.12, 48.99 and 47.72 FPS. Two 1080p runs: demand-to-submit p50 6.094/14.686 ms, p95 14.860/21.180 ms; native GPU upload/shader/Present p50 0.865/0.907 ms; maximum sampled RGB error 3/255. All complete runs matched native origin and ordered receipts; no encoder full-pixel CPU readback. Cancellation after 1/10/30 submissions: actual exit 0, source delivered 3/10/30, viewer exactly 1/10/30, both workers joined. No further decode after the requested count. Three native pre-encoded H.264 v9 regressions passed at 540/720/1080, 1080p 180/180 at 58.36 FPS wire; 1080p local presentation 180/180 at 55.01 FPS; static zero idle readback and geometry rejection passed. Evidence: benchmarks/wgc-live-build-gates-20261126.txt, wgc-live-loopback-release-20261126.json, wgc-live-regression-release-20261126.txt. New preview artifacts in dist: intralink-windows-x64-wgc-live-preview.exe, intralink-0.22.0-wgc-live-locked-source.zip and SHA256SUMS-wgc-live.txt. Prior previews retained. This is a short, same-machine authenticated loopback with a generated window and separate trusted origin tokens; not arbitrary remote AVC, physical monitor sharing, scan-out timing, sustained real-network 1080p60 or production certification. Next priority is decoder process isolation and hostile-input containment before enabling locally authorized physical H.264 Host/Viewer. Driver failure/cancellation under hung native calls, multi-device physical qualification, QSV and other platforms still open.

### Supervised native decoder — Release gate completed

2026-11-26: gate PID 27340 completed exit 0 in 464.90 s (cold optimized dependency build 5m51s). All three strict Clippy feature combinations, rustfmt and the 156-test Debug workspace gate passed. Release binary 15,264,256 bytes, timestamp 19:29:58 local. Isolated decoder Release verifier: all seven parent/child runs exit 0, four complete runs present 480/480 frames, cancellation at 1/10/30 submits exactly requested counts. Steady native ACK rates 540p 55.56, 720p 61.05, 1080p 58.65/58.65 FPS; total transport including initialization/end 21.13/28.52/45.82/49.72 FPS. 1080p demand-to-ACK p50 9.551/6.417 ms and p95 19.665/18.668 ms. Other simultaneous machine builds occurred; these are short uncontrolled loopback observations, not sustained performance or physical network claims. Non-isolated seven-run regression also passed (480/480 complete Present submissions, all cancellations); pre-encoded v9 native 1080p 180/180 at 57.96 wire FPS. Abrupt owned child termination Release test passed: parent exit 1, capture resources released, no false success, 14 delivered/encoded before loss, no orphan. Evidence: benchmarks/decoder-process-build-release-20261126.txt, decoder-process-release-20261126.json, decoder-process-thread-regression-20261126.json, decoder-process-fault-release-20261126.json. Prior archives preserved. Local implementation commit 033a8d3; fault verifier commit 30e1eee. Normal v8 sharing remains unchanged; this Job Object is process lifetime containment, not a restricted-token sandbox. Next: restricted child token/resource access, adversarial codec qualification and actual locally consented physical H.264 sessions, then physical multi-host performance. The 1080p60 sustained target remains unqualified.

### Decoder hardening — Release qualification (2026-11-26)

PID 9236 completed exit 0 (222.26 s). Rustfmt, all three strict Clippy feature combinations, 157 workspace tests and locked Release passed. Release executable 15,315,456 bytes, timestamp 20:18:52 local. Real token-only diagnostic passed in both Debug and Release: parent integrity 0x2000, reduced child token 0x1000, privileges 5 -> 1, three exact restricting SIDs, mandatory no-write-up, parent unchanged. All seven hardened standard isolated-decoder Release scenarios passed verified OS policy marker and distinct parent/child PIDs with actual exit 0. Complete runs presented 480/480 frames; cancel after 1/10/30 submissions passed. Steady native ACK rate at 540p 59.42, 720p 60.04, 1080p 58.76/58.77 FPS; full 1080p transport including initialization/end 50.88/49.66 FPS, demand-to-ACK p50 11.024/6.066 ms and p95 19.101/15.011 ms. The abrupt owned-child loss test passed, source delivered 15/encoded 16, child exit 15 captured, parent error and resource release observed. Pre-encoded v9 1080p regression 180/180 at 50.15 wire FPS; these uncontrolled short loopback timings are not sustained performance certification. The final strict-token graphical Release run independently reproduced parent exit 1 / child 0xc0000142 before Rust entry, no READY, no successful stream or fallback. Scope: hardened standard process qualified locally, strict low-integrity GPU path still experimental and NOT qualified. No hostile remote bitstream, full sandbox or physical H.264 sharing enabled. Source commit 5950d11. Evidence: benchmarks/decoder-hardening-build-gates-20261126.txt, decoder-hardening-release-20261126.json, decoder-hardening-fault-release-20261126.json, restricted-decoder-release-20261126.json. Preserve all prior previews. Next work: investigate headless restricted decoder startup, split native decoding from presentation, validate cross-process GPU resource exchange, actual resource denial and hostile input, then locally authorized physical devices. Do not modify global desktop ACLs or lower the token restrictions to conceal incompatibility.


### Restricted headless fixture — Debug (2026-11-26)

Six embedded generated 1080p access units decoded under low integrity with six ordered immediate display callbacks and sampled YUV max error 1. Native map-and-sample p50 1.808 ms. This local metric excludes parser, initialization, network, cross-process transfer, presentation and scan-out. It is not FPS or end-to-end latency qualification. See `SECURITY.md` and fixture provenance.


### Restricted headless decoder release measurements (2026-11-26)

Trusted embedded 1080p fixture: 6/6 native callbacks under reduced token, YUV max error 1, map/sample p50 1.812 ms (not display or end-to-end). Standard isolated 1080p two local WGC runs: steady decode-submit 58.93/58.85 FPS, wire total 50.74/50.53 FPS, demand-to-submit p50 5.878/6.095 ms, p95 18.364/16.584 ms. In-process regression: 58.79/58.88 FPS steady, 50.50/50.51 FPS wire total. Pre-encoded 180-frame loopback 56.97 FPS. Short generated-window measurements only; no sustained 60 FPS or physical-network qualification. See benchmarks/restricted-headless-native-release-20261126.txt.


### Adaptive v10 independent WGC Release qualification (2026-11-26)
Gate PID 16504 completed with actual exit 0 in 328.40 s. Locked WGC Release compiled in 3m26s; feature graph has no mini-cuda/mini-nvcodec. Actual CLI help passed. Native generated-window 1080p Compact tests both exited 0, 60/60 samples: persistent WGC + CPU resize/codec 49.31 capture FPS, 46.02 changed FPS, capture p50 10.635 ms, encode p50 8.286 ms, local pipeline p50 19.838 ms, readback 1503601920 bytes. WGC + D3D11 scale + CPU codec 55.60 capture FPS, 52.82 changed FPS, capture p50 6.096 ms, encode p50 2.062 ms, local pipeline p50 8.957 ms, readback 497145600 bytes. Both retained one staging and one RGBA allocation; scaled path retained one processor. These fixed-geometry generated-window runs test the optional vendor-independent native capture and CPU codec, not physical remote v10 automatic adaptation, sustained 60 FPS, scan-out timing or native H.264. Previous GDI 1080p local baseline 2.30 FPS was a separate diagnostic run. Evidence benchmarks/adaptive-v10-validation-20261126.txt. Artifact dist/intralink-windows-x64-adaptive-v10-wgc-portable-preview.exe (14657024 bytes), SHA256 c3f119468500f8b993a6d6d687e3599d5dcf81349a61d0b0f6312b7f150c40e9. Source implementation commit a986fdf. All automated gates and native fixture checks passed; physical sessions, scaler retargeting, codec capability negotiation and platform matrix remain open.


### Negotiated v11 and live WGC retarget - Release qualification (2026-11-26)

Gate PID 29644 completed with actual exit 0 in 218.55 s. Standalone 87 tests, portable workspace 169 tests, NVIDIA-feature workspace 182 tests, strict Clippy CPU/WGC/NVENC and locked WGC Release passed. WGC feature graph excludes mini-cuda and mini-nvcodec. Native retarget diagnostic passed all 30 samples in one owned-window capture session at 1080/720/540/720/1080, with native source 3336x1878 and fitted output 1918x1080 / 1279x720 / 959x540 / 1279x720 / 1918x1080. Six-frame readback bytes per tier: 49714560 / 22101120 / 12428640 / 22101120 / 49714560; 540p is exactly 75% lower readback volume than 1080p in this fixture. Total readback 156060000 bytes, five native processors, five staging allocations, one reused CPU RGBA allocation. Native source/device/context/geometry rejection passed, as did static capture, native close, cancellation and readback-after-stop rejection. Artifact: dist/intralink-windows-x64-negotiated-v11-wgc-portable-preview.exe, SHA256 9c8f5694c0ee2848ca7423b28990455c975d519d1a3b8188597582bac67dfac4. Evidence: benchmarks/negotiated-v11-retarget-validation-20261126.txt. Implementation 1162f9c. Native diagnostic uses generated local window and forced tier transitions; not physical remote adaptation, sustained 60 FPS, scan-out latency or hardware codec support. Keep CPU baseline, explicit v11 opt-in and all existing consent and native decoder isolation policies. Physical sessions, per-viewer resolution/FPS bounds, mixed-version workspace, platform-specific hardware codec negotiation and full hostile native bitstream containment remain open.
