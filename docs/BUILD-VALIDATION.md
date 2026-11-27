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


### Preview 3 final native gate (2026-11-27)

The preserved Windows x64 binary (SHA-256 `6a655a71b34b514c81f610f25a80ee5edd63f3141ef8175023b9313ffa233dc0`, 15,633,408 bytes) passed all six native gates without concurrent Rust builds at the gate boundaries. PID 20060 exit 0, diagnostic time 34.00 s. Evidence: `benchmarks/input-motion-queue-native-final-20261127.txt`. Pipeline, visual, capture-lock, staged-readback, retarget (30 samples) and geometry all exited 0. Four visual checks compared 491,443,200 pixels in 237 changed frames; maximum channel MAE 1.1696, maximum block MAE 5.2227, zero large-error fraction. Generated owned test window and authenticated local QUIC only. Ordinary serial changed FPS 40.31/44.77, overlap 44.09/43.06; bounded lock 47.63/49.94 versus ordinary 41.56/45.57; staged 46.43/45.84 versus blocking 47.48/49.35. No reproducible promotion evidence, no sustained physical 1080p60 or physical KVM qualification. Prior automated results are unchanged; no rebuild. Preserve blocked attempts and earlier releases. Windows unsigned private-LAN development pre-release is eligible for publication after archive and remote checks.

Next: Windows/Linux/Android release matrix. Use BASE (Controller) and CLIENT (Host) as simple user-facing roles; preserve durable vault identity and explicit independent grants, and do not enable global hooks or unattended control. Linux X11 native desktop build remains unqualified; Android application is absent. Qualify physical two-device KVM before global edges. No simultaneous Hybrid multiplexing yet. Only one Desktop Commander machine was available. Maintain the portable software video path and existing timer/adaptive-media gates.

### Portable candidate CI checkpoint (2026-11-27)

Preview 3 Windows publication verified at https://github.com/willerxar-cmyk/IntraLink/releases/tag/v0.22.0-preview.3 (tag source 6f6ae57; five remote asset digests matched the local manifest and files). Previous hosted CI run 45999798075 used Rust 1.100.0 and failed strict Clippy on the new chunks_exact_to_as_chunks lint; the Linux desktop link also failed with missing -lgbm. Local qualification used rustc/Clippy 1.96.0. Pin CI to the verified 1.96.0 toolchain and add libgbm-dev to the two Linux desktop dependency gates. Keep warnings denied; adaptation to newer Rust/Clippy is still open and must be tested rather than suppressed. Restrict automatic push CI to main to avoid duplicate tag-triggered builds; PR and manual dispatch stay available.

Manual Portable desktop candidates workflow uses independent Windows x64 (WGC/input) and Linux x86_64 (desktop-preview) jobs. Each checks locked portable dependency graph, strict Clippy, tests, release, actual CLI version, per-platform Cargo notice inventory, package integrity and SHA-256. Linux also checks dynamic library resolution. No automatic public release, native capture or physical KVM assertion from hosted CI. The Windows packager was exercised against the existing qualified binary and notice archive; YAML parsed locally. Linux packaging and dependency inventory require actual hosted execution. Android SDK/NDK are installed on Windows, but no Android application source exists and signing has not been qualified. Ubuntu 24.04 WSL lacks Rust and pkg-config; local sudo is blocked by Desktop Commander policy. No local privilege changes or other-project builds interrupted. Keep Windows preview.3 assets unchanged.

Next: inspect candidate workflow outcomes, resolve any actual Linux build or notice failures, test Linux X11 and Wayland on real authorized devices; implement genuine Android Kotlin integration backed by Rust, and persistent graphical BASE/CLIENT onboarding without granting reciprocal permissions. Existing profile save deliberately rejects role replacement; design explicit authenticated role migration preserving identity, compatible grants, replay receipts and invitation revocation before permitting changes. The current release still has CLI-only setup.

### Desktop onboarding and Linux candidate checkpoint (2026-11-27)

Previous main CI run 46020301355 passed. Portable run 46020325529: Windows candidate complete; Linux strict lint, tests, native Release, actual CLI version and ldd all passed. Linux packaging failed only on missing cookie-factory-0.3.3 notice; fixed exact upstream tree traversal for LICENSES and REUSE metadata, repository subdirectory provenance and cross-host Cargo metadata. Local cross-host Linux dependency inventory succeeded with 375 packages and 672 notice files (not native-host inventory). drm-fourcc 2.2.0 original crate/commit lacks a separate MIT file; preserve original manifest and README and the exact later upstream MIT permission blob with explicit independent-review marker. Do not claim original copyright was present. Final native Linux package must use native Linux build-host inventory; preserve build-only CI artifacts before packaging.

Desktop onboarding implements BASE/CLIENT chooser for desktop-preview and interactive terminal-only fallback; named identity stored inside encrypted vault. Explicit authenticated role migration preserves identity, name, compatible grants and replay receipts, revokes incompatible grants and cancels pending invites; no automatic authority. Old readers may reject changed vaults; no downgrade. CLI and in-memory render tests passed. Standalone vault 13/13 passed; first strict Clippy caught three style diagnostics, fixed with no suppressions. Retry PID 20644 exit 0 (285.25 s): portable strict Clippy, workspace 248 tests, CLI-only strict Clippy and 11 tests, locked Release, fmt all passed. Cargo.lock unchanged for this desktop checkpoint. Logs benchmarks/onboarding-validation-20261127.txt and onboarding-validation-retry-20261127.txt. Preserved Windows binary dist/intralink-windows-x64-onboarding-portable-preview.exe, 15,684,096 bytes, SHA256 d36ec2be1c3ee02f2d3e12dac2bdb947a843017ef024837138102d10d602885f. Exact setup/set-role/rename-device help passed. Native video regression not yet run; concurrent external Rust builds detected (2 cargo, 15 rustc). Do not publish this candidate until six native video gates pass and preserve old preview.3. Physical KVM and interactive role chooser qualification pending. See ONBOARDING.md.

### Preview.4 desktop native and packaging checkpoint (2026-11-27)

Windows source checkpoint 3b62c17 (unchanged desktop code and lockfile at build time). Preserved onboarding executable SHA256 d36ec2be1c3ee02f2d3e12dac2bdb947a843017ef024837138102d10d602885f, 15,684,096 bytes. Native gate PID 14376 exited 0, 35.31 s diagnostics; pipeline, visual, capture-lock, staged-readback, retarget (30/30) and geometry passed. Evidence benchmarks/onboarding-native-20261127.txt. Four full-pixel comparisons: 236 frames / 489,369,600 pixels; max channel MAE 1.1551, max block MAE 5.2227, no channel-error fraction above the defined large-error threshold. Local generated-window authenticated loopback only. Ordinary serial changed FPS 31.62/43.68, overlap 44.33/44.96; bounded lock 45.44/47.42 versus ordinary 42.06/44.98; staged 46.94/44.48 versus blocking 49.16/48.69. No sustained 1080p60 proof or policy promotion. Prior 248 workspace tests, 13 vault tests, 11 CLI-only tests and strict Clippy remain valid.

Linux CI job 140591810705 of run 46024361648 passed strict lint, tests, native release, CLI version, ldd, native-host license inventory, archive creation and integrity. Downloaded artifact dist/preview4-linux-ci-46024361648/intralink-0.22.0-linux-x64-3b62c172e502-candidate.tar.gz, 7,898,745 bytes, SHA256 b930ed269cbc3b33f2c7a9f85a8052c27cf5dddc41450f71c326812e024b1db7. All 687 archive entries checked against internal manifest; binary 18,947,136 bytes, SHA256 075261f0c04d10391088d010cd64ddb2f31be9f655760a8a539102c25c8c23ad. Inventory 374 dependencies / 670 original notice and supporting files. Preserve provenance-review markers for drm-fourcc 2.2.0 and enum-assoc 1.4.2; no independent compliance certification. Physical capture/input and Wayland are not qualified; Linux has no KVM input adapter.

Preview.4 desktop public packaging may use these exact qualified binaries, with the final documentation/source archive and updated manifest. Source tags must exclude unvalidated local Android changes. Keep preview.3 assets and tags intact. Windows chooser rendering/CLI/vault tests passed, but actual interactive setup and physical KVM remain unqualified. No new video-path change. Next: finish Android local setup packaging, strict target lint, license inventory and physical runtime validation, then implement permissioned Linux input adapters and device dashboard. Android signed release remains conditional on authorized signing and actual qualification.

### Preview.4 publication and Android local onboarding checkpoint (2026-11-27)

Public preview.4 Windows/Linux release verified at https://github.com/willerxar-cmyk/IntraLink/releases/tag/v0.22.0-preview.4 (tag commit 07302ef, native desktop source 3b62c17). Six uploaded assets and remote digests verified; Windows portable ZIP SHA256 b32ca36122c7407dd51a4cbb1e4e1119d7bc5b4e0311cdd064226f21d50e77b4; Linux TAR.GZ SHA256 3e5e9cb40cacc7bc22150ce2c23c8d0b9bf9a8dd2433fb243bc633bc333557a9. Final packages carry updated docs and notices. Keep all releases immutable; Android source not in the desktop tag.

Android module apps/android and crates/intralink-android use Kotlin + real shared Rust vault JNI. BASE/CLIENT selection, peripheral detection/recommendation, named persistent identity, unlock, explicit role migration and rename. Local only: no network permission or remote session. App-private encrypted vault, bounded passphrase handling, FLAG_SECURE, explicit backup/transfer exclusions, resource-based UI strings, safe window insets and native vector icon. Optional existing-keystore signing through four INTRALINK_ANDROID_* environment variables; no release key created or used. Cargo.lock only adds the internal Android bridge package, using existing jni 0.21.1; no existing registry version changed.

Initial native ARM64/x86_64 builds succeeded (android-native-onboarding-20261127.txt); Kotlin primitive-array conversion failure fixed (android-gradle-onboarding-20261127.txt). Debug/Release Kotlin, APK Debug, unsigned APK Release and unsigned AAB Release passed (android-gradle-onboarding-retry-20261127.txt). Hardened UI/backup plus actual JNI instrumentation APK built (android-onboarding-hardening-20261127.txt). Strict target Clippy and Release both ABIs exit 0, PID 24944 (167.14 s), android-strict-native-20261127.txt. Final Gradle + validator PID 24400 exit 0, android-final-candidate-20261127.txt: both packaged ABI ELFs and generated-library JNI exports, 16 KiB LOAD alignment, archive integrity, zipalign, actual package/version, no Android permissions, debug/test signatures passed. Release APK/AAB unsigned; lint retains one unsuppressed AndroidGradlePluginVersion advisory (9.3.1 pinned). Original notice provenance review remains open. ARM64 263 native dependencies / 476 notice files; x86_64 264 / 478; JVM exact two Maven artifacts, Kotlin stdlib 2.2.10 and annotations 13.0 (annotations provenance review marker). No connected Android device/AVD/system image found; JNI instrumentation compiles, has not run. No APK/AAB public distribution. Candidate directory dist/android-onboarding-candidate-20261127; SHA256SUMS.txt verified.

- intralink-android-onboarding-debug.apk: 2518901 bytes, SHA256 57a64a9beda286d0fdf48072432706bad06b7b948ed39f1f3f57ff312999e401.
- intralink-android-onboarding-instrumentation.apk: 13894 bytes, SHA256 919a9551613f14ea6eb50f350496013933f1eef2325a0bc1f3c64e17610950c6.
- intralink-android-onboarding-release-unsigned.aab: 2276254 bytes, SHA256 3a5e34af91dd9ef852dc0015fbc704479884af81c76ec047ca979b0adbebb610.
- intralink-android-onboarding-release-unsigned.apk: 2309441 bytes, SHA256 9f04f33bed777bcf3c12c115d239d40b10cbd4a1cd91a0c1bd68b1532586e934.

Next: execute real Android instrumentation and first-run UI on a consented device/emulator; configure an authorized distribution key and resolve the Gradle advisory and license review before public Android release. Implement explicit Android pairing and shared QUIC transport, independent capability sessions and dashboard; then MediaProjection/MediaCodec and permissioned input. Implement Linux X11 input source/sink with visible consent and fail-closed release, then Wayland via compositor portals. Qualify two real machines before physical monitor edges and expand to eight; preserve CPU video path and existing adaptive/timing gates. See ANDROID.md, ONBOARDING.md and INPUT-SHARING.md.
