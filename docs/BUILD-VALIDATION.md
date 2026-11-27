# Reproducible validation from a Rust-equipped machine

The source archive does not contain a fabricated Cargo.lock or build artifacts. Install the official Rust stable toolchain with Cargo, rustfmt and Clippy on a development machine. Resolve the first dependency graph, review it and commit Cargo.lock. Existing optional native desktop/VA-API prerequisites are documented in [DESKTOP.md](DESKTOP.md) and [HARDWARE.md](HARDWARE.md). Review dependencies before executing. No remote operation or privileged service is started by the validation runner.

From the extracted repository root:

```sh
rustc --edition=2024 -D warnings tools/validate.rs -o intralink-validate.exe
./intralink-validate.exe --resolve --resources
```

The executable extension works on Unix and Windows; PowerShell may also use `.\intralink-validate.exe`. For Windows/Linux add `--desktop` after installing native build requirements. Add `--vaapi` on Linux only with libva development dependencies. Omit `--resolve` after committing the reviewed Cargo.lock. The runner does not automatically install dependencies, alter rustfmt configuration or overwrite an existing lockfile. Cargo may fetch dependencies required by its normal operations.

The runner checks compiler and Cargo versions, compiles and executes the actual registry-independent Rust test harness, checks formatting, resolves only a missing lockfile when explicitly allowed, checks locked dependency metadata, runs workspace Clippy with `-D warnings`, tests and release builds. Requested optional feature gates run independently. No stale test executable is run if compilation fails. A failure in formatting or Clippy does not hide subsequent diagnostics. Any failed or blocked gate returns a nonzero exit. Preserve terminal output for review; redirect it to a local log if desired.

Once local automated checks succeed, execute the security, native consent, direct-network, GPU, cancellation, throughput and platform tests in [VALIDATION.md](VALIDATION.md). The runner does not establish device compatibility, audit cryptography, test real capture or prove a GPU encoder works. Hosted CI is defined, not executed by this source archive.

When multiple supported feature flags are requested, the runner also checks them together. CI adds combined Windows desktop/resource and Linux desktop/resource/VA-API builds, tests and Clippy gates. The individual feature gates remain in place to isolate failures. No CI run or lockfile resolution was performed here.

### Isolate build outputs on a shared development machine

PowerShell: set `$env:CARGO_TARGET_DIR = Join-Path (Get-Location) "target"` before running Cargo or the validation runner. This avoids waiting on another project using a globally configured target directory. The v0.19 Windows validation was limited to an isolated workspace containing `intralink-core` and the actual overview renderer used by the portable harness. Its dependency resolution cannot substitute for the complete workspace lockfile.


### Mandatory independent software media gate (2026-11-26)

Run the Windows/Linux `desktop-preview,resource-monitor` build, Clippy with `-D warnings` and workspace tests without `nvenc-preview`; inspect `cargo tree --locked -p intralink --no-default-features --features desktop-preview,resource-monitor -e normal` to ensure no mini-cuda or mini-nvcodec dependencies. On Windows execute the actual generated-window `desktop-media-check --height 1080 --samples 60 --codec compact` and lower-resolution alternative with the CPU-only codec executable. The native OS capture path may still use platform graphics APIs; no NVIDIA codec runtime is required. Preserve a separately named portable Release artifact, and validate optional accelerated builds separately. The dependency-free media harness now includes the H.264 protocol and portable quality policy modules; the local overview banner uses a standard-library error instead of a registry dependency, preserving actual source coverage.


### Adaptive geometry (v10)
Include `adaptive` and `presentation` in the dependency-free media harness and execute both CPU-only and optional NVIDIA-feature workspace suites. Run strict Clippy for CPU, WGC and NVENC feature combinations. Check the independent `wgc-preview,resource-monitor` dependency graph for absence of mini-cuda/mini-nvcodec before native generated-window diagnostics. Preserve the actual v8/v9 tests. v10 tests must cover resolution transitions, full-keyframe reset, old immutable snapshot lifetime, malformed initial/transition records, raw invalid peer geometry before ACK, wrong identity and incompatible ALPN. Keep physical multi-device sustained performance as a separate gate. See [ADAPTIVE-MEDIA.md](ADAPTIVE-MEDIA.md).


### Negotiation and retargeting validation

The mandatory independent harness includes capabilities mask and deterministic intersection tests. Run authenticated v11 loopback and adversarial handshake tests, CPU and NVIDIA-feature workspace suites, strict CPU/WGC/NVENC Clippy, and locked WGC Release without CUDA dependencies. On Windows, run `desktop-retarget-check`, `desktop-geometry-check`, and `desktop-static-check --height 1080 --gpu-scale`. Preserve native output transition, capture identity, teardown and strict readback geometry checks. Physical remote sessions remain separate.


### Bounded v12 validation

Run the portable harness and locked CPU/NVENC workspace tests with strict CPU/WGC/NVENC Clippy. Include exact profile intersections, hostile limit/selection cases, per-slot override and CLI tests, and three independently paired loopback hosts using different negotiated resolution/FPS ceilings. Validate WGC Release without NVIDIA dependencies and retain existing protocol/security regressions. Physical sessions are a separate gate.


### Runtime capability gate

Run calibration pure-policy and generated-codec tests, actual CLI validation, independent CPU/NVENC workspace suites and strict CPU/WGC/NVENC Clippy. Build locked independent WGC Release; run `desktop-profile-check --slots 1`, `--slots 3`, `--slots 8`, and `--hardware`. Record actual timings, observed recommended ceilings, per-adapter configuration and conversion outcomes separately. No timing-sensitive unit test assertions; physical streaming remains separate. Concurrent calibration additionally requires complete worker/sample counts, real mixed-resolution decode tests, malformed warm-up cancellation without a rendezvous hang, and 1/3/8-worker Release measurements.


### Host calibration gate

Qualify the pure selection policy (manual ceilings, duplicate/invalid measurements, CPU-only candidate), actual dense encoding/decoding, cancellation and explicit CLI opt-in. Run strict Clippy, portable and NVIDIA feature suites, locked WGC Release without NVIDIA dependencies, `desktop-profile-check --host` and owned-window retarget/geometry regression. Record actual candidate results and selected profile; do not imply physical monitor/network qualification.


### JPEG optimization gate

Run the standalone media harness, locked workspace CPU/NVENC tests, independent no-default-features scalar app tests, and strict CPU/WGC/NVENC/scalar Clippy. Verify identical baseline 4:4:4 sampling and quantization tables and bounded decoded error at qualities 35/75/90. Check partial-record rejection, buffer reuse, lossless budget subset and smallest-representation invariants. Build independent scalar and SIMD WGC Releases; run `desktop-profile-check --encoder` on each, then the SIMD Host calibration and native retarget diagnostic. Preserve existing artifacts and record separate encoder-only and local full-pipeline measurements. No sustained network claim.


### Parallel JPEG gate

Run the standalone harness, strict CPU/WGC/NVENC parallel and independent scalar Clippy, full parallel CPU and NVENC workspace tests, and scalar app tests. Verify exact serial/parallel decoded pixels at every supported tier and three qualities, bounded strip-buffer reuse, generated-header validation and atomic oversize rejection. Build locked WGC SIMD parallel Release; execute encoder comparison and stage profiling, Host calibration and native retarget diagnostics. No change to network ordering or claims of sustained physical FPS.


### Ordered Host pipeline gate

Validate FIFO pending requests, shared credit reservation and record budget, initial-frame and priority constraints, authenticated QUIC ordering and saturated credit, unauthorized identity, local two-frame lane and exact delta base, cancellation/join and CLI restrictions. Run locked CPU/NVENC parallel workspace and scalar app suites, four strict Clippy configurations, standalone harness, WGC portable Release and owned-window native pipeline, retarget and geometry checks. No physical network or sustained FPS claim. See [ORDERED-PIPELINE.md](ORDERED-PIPELINE.md).


### Visual fidelity gate

Run generated visual scene/marker, deliberate RGB swap/local block corruption/lossless mismatch, real software codec and CLI conflict tests. Qualify strict CPU/WGC/NVENC/scalar builds and locked portable WGC Release. Run `desktop-profile-check --visual` (full-canvas reference comparison) and separately `--pipeline` (calm scene performance), with no high-entropy visible pattern. Regress native retarget/geometry checks. Record actual decoded pixel counts and errors; do not compare old dense and new calm FPS directly. See [VISUAL-FIDELITY.md](VISUAL-FIDELITY.md).


### Capture-start grant gate

Test early, duplicate, canceled and missing capture-start reports, actual delayed start driving the next absolute deadline, one pending local grant, independent FPS cap, FIFO ordering and saturated credit. Qualify locked CPU/NVENC workspaces, independent scalar application, strict four Clippy configurations, portable WGC Release. Compare preserved baseline and new calm-scene runs, new ABBA stage traces and separate full-pixel visual/retarget/geometry diagnostics. See [CAPTURE-GRANTS.md](CAPTURE-GRANTS.md).


### WGC capture timing and lock experiment gate

Retain every capture-start grant gate. Add `desktop-profile-check --capture-lock` to run the controlled same-pipeline ABBA lock-policy comparison and separate full-pixel regression of the experimental policy. Verify the command is explicit and conflicts with other diagnostic selectors, timing counts agree with actual attempts/readbacks, and disjoint timing intervals fit inside their enclosing stage. Native `Map` timings include synchronization; no isolated GPU execution claim. Do not adopt the experimental wait automatically. See [WGC-CAPTURE-TIMING.md](WGC-CAPTURE-TIMING.md).


### Bounded staged readback experiment (2026-11-27)

Run the full previous gate, including capture-lock, plus `desktop-profile-check --staged-readback`. The native geometry check now exercises staged exact RGBA, pending overwrite/reset rejection, duplicate completion, staging reuse and fresh readback after dropping a pending copy. The comparison uses four 180-record runs and separate staged full-pixel visual validation. Record nonblocking Map polling and interpoll delay separately. Keep the Host synchronous until experimental behavior is separately qualified. See [STAGED-READBACK.md](STAGED-READBACK.md).

## Input preview build gates

The opt-in Windows input-preview crate must pass strict Clippy and actual workspace tests including input-preview,wgc-preview,jpeg-parallel,resource-monitor. Standalone core tests include input protocol/state/layout; actual QUIC tests cover independently authenticated three/eight hosts, release ordering, wrong peer, malformed sequence, rejection and disconnect release plan. Native scan mapping unit tests construct INPUT values without injecting into the developer desktop. Preserve CPU, NVIDIA, WGC and scalar gates; check that the portable input Release dependency graph contains neither mini-cuda nor mini-nvcodec. Run input --help, input host --help and input workspace --help on the exact Release binary. Physical input acceptance is not inferred from these tests. Preserve native video regressions and benchmark logs.


### Input motion queue gate

Run `cargo test --locked -p intralink-transport input -- --nocapture` in addition to affected workspace and strict Clippy gates. Require exact retained input order, no drag/chord coalescing, bounded saturation, expiry, canceled receive, actual QUIC acceptance cancellation and the three/eight-host regression. Local ACK timing is not physical input-to-display latency. Do not benchmark during concurrent builds. See INPUT-QUEUE.md.
