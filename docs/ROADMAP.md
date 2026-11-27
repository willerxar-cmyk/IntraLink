# Implementation milestones

| Version | Scope | Exit gate |
|---|---|---|
| 0.1 | Direct connectivity diagnostic, manual identity exchange and DDNS update | Build, formatting, Clippy and unit/loopback tests; confirm zero unwanted relay traffic |
| 0.2 | Persistent local identities, encrypted local vault, manual bilateral diagnostic pairing and local revocation | Build, encrypted vault/revocation tests; manual pairing and DNS negative tests |
| 0.3 | IPv4 LAN discovery, dual-stack DNS, signed invitations, route diagnostics, retries and DDNS refresh; native keystore, NAT signaling and relay remain open | LAN, IPv6, NAT and CGNAT matrix; visible path state and no implicit relays |
| 0.4 | Windows/X11 authorized screen viewing (experimental source delivered); input and GPU encoder still open | Native consent flows, measured latency and resource consumption, user-visible stop |
| 0.5 | Desktop v2 efficiency, bounded pipeline, local cancel and measurements (source delivered) | Compile; validate cancellation, throughput, memory and compatibility |
| 0.6 | Desktop v3 bounded receipt credit and local codec feasibility tool (source delivered) | Compile; delayed/malformed receipt and network performance tests; hardware capability tests |
| 0.6.x | Native macOS and Wayland host, hardware media engine and unified desktop interface | Native permissions, negotiated codecs, measured latency and signed installers |
| 0.7.x | Explicitly shared file transfers, system resources, text editor and log watch | Path confinement and authorization tests, integrity/resume, bounded monitoring |
| 0.8 | Authorized terminal and audio, Android/iOS controller and supported mobile host features | Restricted privileges, mobile lifecycle tests, store policy review |
| 1.0 | Release engineering, device matrix, recovery, documentation and independent audit | All advertised capabilities tested; signed packages; no open critical security bugs |

No fake implementations or simulated success states. No hardcoded credentials. Code and comments in English and UTF-8. Keep modules small, use idiomatic Rust, no unsafe Rust in portable business logic; isolate and audit necessary OS FFI. Treat warnings as failures. Add a module only with its own functional and negative tests. Version API and vault migrations. Keep the project separate from any unrelated repository.

## v0.3 handoff

Source delivered for 0.3. The exit gates for 0.1–0.3 remain open until Rust execution and security tests. Native keystore support, live revocation, session capability negotiation, authenticated signaling/NAT traversal and optional relay are not delivered. The new offline enrollment does not open a network enrollment endpoint; LAN discovery is opt-in and limited to IPv4 private addresses. No screen sharing or remote-control feature is shipped. Next: resolve and compile dependencies, fix diagnostics and formatting, commit a resolved lockfile, validate invitations/replay and LAN, then implement native key protection and session authorization before screen capture/input. Do not treat source milestones as verified release milestones.

## v0.4 handoff

The first read-only desktop source is delivered. Do not close the milestone until native build/tests, identity/consent and disconnect tests, independent UI event handling and performance measurements are complete. Priority: resolve Rust compilation and lockfile, run existing tests and the optional feature matrix, fix diagnostics, verify on real Windows and X11 displays. Then native permissioned Wayland/macOS capture, hardware codec negotiation and explicitly scoped input, with local stop and revocation. NAT traversal, native keystore, persistent session audit, full GUI, file/resource modules and mobile packages remain open.

## v0.5 handoff

Prioritize compiling and running the accumulated code before adding new privileged capabilities. v0.5 intentionally focuses on the desktop data path and cancellation rather than macOS/Tauri expansion. The v2 desktop wire protocol replaces v1 and both endpoints need updating; existing vaults and probe wire format are preserved. Native capture/codec still block the UI thread while executing; ACK-per-record trades high-RTT throughput for bounded latency/backlog. A production GPU codec, authenticated NAT traversal, native keystore, live revocation, permission model, authorized input, native platforms and full GUI remain open. Run [PERFORMANCE.md](PERFORMANCE.md) and [VALIDATION.md](VALIDATION.md) before claiming performance or shipping.

## v0.6 handoff

The desktop v3 wire protocol replaces v2 with fixed three-record receipt credit. Both endpoints must upgrade. Vault and diagnostic probe are unchanged. This milestone was inserted before the originally planned file/resource modules because network pacing and codec validation remain open. The local synthetic FFmpeg tool is executed for four CPU profiles; it is not an integrated media backend. Native Rust execution and hardware validation remain the release gate. See [CODECS.md](CODECS.md) and [VALIDATION.md](VALIDATION.md).

## v0.7 handoff

The desktop v4 protocol requires both peers to match JPEG or hybrid Rust tiles/JPEG. The pure Rust tile encoder/decoder and bounded ordered receiver are implemented in source, not run. No new dependencies; vault and probe unchanged. The Rust microbenchmark is included but unexecuted. Native hardware codec integration, a separate decode/capture worker, NAT traversal, native keystore, consented input, files, mobile and unified GUI remain pending. The first release gate is still a real Rust build, formatting, Clippy, tests and native session/security validation. See [TILES.md](TILES.md) and [VALIDATION.md](VALIDATION.md).

## v0.8 handoff

The compact tile codec, explicit v5 mode selection, standalone Rust test harness and comparative benchmark are delivered as source. The current hardware acceleration milestone has not been closed. First execute the dependency-free media gate, then the full workspace and desktop checks. Next implement and validate platform-native hardware codec adapters with explicit capability discovery, then permissioned Wayland/macOS, unified UI and remaining modules. Do not mistake computed payload savings for measured session performance. Both desktop peers require v0.8; the vault and diagnostic probe are unchanged.

## v0.9 handoff

Complete lossless Compact bases and v6 protocol are delivered as source, with no new dependencies. The 128 KiB attempt policy is provisional and must be benchmarked. Before expanding privileges or shipping, compile and execute all accumulated tests, resolve formatting/API issues and lock dependencies. GPU adapters (NVENC, AMF/VAAPI, QSV, VideoToolbox, MediaCodec), native permissions, actual NAT traversal, native secure storage, full management GUI, file/resource modules, audio and mobile packaging remain open. Do not claim hardware acceleration from a CPU codec.

## v0.10 handoff

Add optional Linux driver capability/configuration reporting and the reusable Rust NV12 conversion primitive before implementing a native hardware encoder. Both preserve the existing v6 wire protocol and vault. CI is split so formatting, dependency-free tests and Cargo builds are independently visible. The release gate remains open; resolve Rust, execute the accumulated tests, review the real lockfile, then perform actual hardware encode/decode and native consent validation. No new remote privileges, GPU streaming, NAT traversal, GUI, files, audio or mobile support are claimed. See [HARDWARE.md](HARDWARE.md).

## v0.11 handoff

The optional local-only resource monitor, checked single-pass letterboxing and native Rust validation runner are delivered as source. No remote resource protocol, process list, GPU metrics or management operations have been added. The next release gate is still the initial reviewed Cargo.lock, real workspace and feature builds/tests, then consented native session and GPU validation. Use [BUILD-VALIDATION.md](BUILD-VALIDATION.md); use [RESOURCES.md](RESOURCES.md) for local sampling and its limitations. Do not add privileged remote operations until the security and native-session gates pass. The v6 wire protocol and encrypted vault are unchanged.

## v0.12 handoff

The direct routing policy and WAN comparison are delivered in source. The media protocol v6, vault and probe are unchanged. The Rust toolchain and registry remain unavailable in the authoring environment. Do not treat this routing change as ICE/STUN/hole punching or a successful network session. First execute the dependency-free core gate, generate and review a real lockfile, fix all diagnostics, and run workspace/feature and native tests. GUI, authorized input, GPU streaming, files, remote resources, native keystore, audio and mobile packages remain open.

## v0.13 handoff

Adaptive static-screen pacing and receive allocation recycling are delivered in source. The v6 wire format, direct routing, permissions and vault are unchanged. Combined-feature validation is defined, not executed. Freeze new network privileges until a reviewed Cargo.lock, real formatting/Clippy/workspace and combined builds, loopback tests, native consent/cancellation and performance checks pass. GPU encoding, native keystore, NAT traversal, unified GUI, input, file management, remote resources, audio and mobile packages remain open.

## v0.14 handoff

Optional local attempt history is delivered in source. It is not a security-grade audit trail and does not provide live revocation or identify the authenticated remote peer. No remote privileges or protocol changes. Keep the release gate open until Rust compilation, the reviewed lockfile, Clippy, real tests, platform consent and cancellation tests pass. See [SESSION-HISTORY.md](SESSION-HISTORY.md). Next priorities are actual compilation, native key protection, capability authorization, native UI and hardware video, then approved remote input/files and mobile support.

## v0.15 workspace milestone

Multi-device view-only scheduling, fullscreen and 60 FPS configuration are implemented as source. Partial Windows core validation passed (13 tests). The release gate remains full Cargo/native tests with three hosts and measured performance. True remote input, monitor switching on a remote host, native hardware codecs, Android/iOS controller and host adapters, NAT traversal, remote file/resource/audio and production packaging remain open. See [WORKSPACE.md](WORKSPACE.md).

## v0.16 presentation milestone

Separate native presentation from the ordered decoder canvas lock with immutable snapshots, add local fullscreen monitor cycling and preserve live sessions on failed window transitions. Six exact core tests and a standalone synthetic CPU microbenchmark ran on Windows. No protocol or dependency changes. Native integration, full Cargo and Clippy gates, remote display switching, input, GPU codecs, NAT traversal, mobile and remaining modules are not complete. Next transfer the complete workspace to the working Rust machine and validate all accumulated gates before claiming production readiness.

## v0.17 overview milestone

Local device overview, click-based selection and activation, and allocation-bounded thumbnail composition are delivered as source. The six new pure Rust tests and six presentation regressions passed on Windows. Native GUI, full workspace build, actual concurrent sessions, hardware encoders, remote input and mobile remain open. Do not treat the overview as a mobile GUI, a full remote-control feature or proof of sustained 60 FPS. Next release gate: transfer the full workspace to the available Windows Rust machine, review and commit the dependency lockfile, fix formatting/Clippy/build/tests, then execute the native acceptance gates in [WORKSPACE.md](WORKSPACE.md). No new privilege or protocol changes.


## v0.18 measurement milestone

Local per-session observations and stable cached overview thumbnails are delivered in source. Five new tests and 34 selected regressions passed in the isolated Windows Rust harness. Still open: full reviewed lockfile/build/Clippy/test matrix, native capture and GUI, actual multi-host sessions, hardware codec integration, measured sustained FPS, explicitly authorized remote input and remote monitor selection, NAT traversal, file/audio/resource protocols and mobile adapters. See [TELEMETRY.md](TELEMETRY.md). No new network privileges or wire changes.

## v0.19 core validation milestone

Complete portable harness, isolated Cargo core library, Clippy and optimized CPU benchmark examples are executed. Keep the actual application release gates open: full dependency graph and reviewed lockfile; workspace formatting, Clippy, tests and release builds; vault and transport verification; native desktop and simultaneous session acceptance; hardware codecs and sustained FPS. Proceed with consented input, remote monitor selection, mobile platforms and remaining capabilities only against these gates. See [CORE-VALIDATION.md](CORE-VALIDATION.md).

## v0.20 vault validation milestone

The actual vault and core crates are transferred and tested in an isolated Windows Cargo workspace. Authenticated save regression checks and isolated Clippy pass. The next release gate is the actual transport crate, including its authorized loopback tests, rejection of wrong identities, separate ALPNs and synthetic multi-host tests; then the native application, full lockfile, formatting/Clippy/build/test matrix and native consent/cancellation. No production readiness or 60 FPS claim.


## v0.21 transport validation milestone

Compile and test the real core, vault and transport in an isolated Windows Cargo workspace. The existing loopback transport suite and the synthetic three-host pairing/media integration pass. Correct the test-only address lifetime errors rather than suppressing diagnostics. The shared-controller endpoint is validated at the library level only; the current native workspace still opens a separate endpoint per session. Next compile the actual application, resolve and review its complete lockfile, run formatting/Clippy/build/test feature gates, and validate local consent, window transitions, physical multi-host streaming and sustained FPS before enabling further remote capabilities. See [TRANSPORT-VALIDATION.md](TRANSPORT-VALIDATION.md).

## v0.22 current engineering gates (2026-11-26)

Full Windows workspace, real loopback multi-host sessions, WGC generated-window lifecycle/geometry tests, explicit read-only Host backend source, native D3D11 NV12 and NVIDIA encoder/decoder synthetic diagnostics have executed. Separate v9 H.264 loopback now verifies native packets over authenticated QUIC while normal sharing remains v8. Optional high-resolution private Windows pacing improves the short diagnostic's observed transfer cadence. These are not production or physical remote-sharing certification.

1. Complete H.264 profile/SPS and decoder hostile-input containment, same-device/cross-adapter WGC interop, cancellable codec worker and ordered native presentation. Preserve explicit foreground consent, identity and local stop.
2. Qualify actual Windows Host/viewer on authorized disposable test displays, static and motion, revocation/stop, display topology and device failure. Measure capture-to-present latency, sustained 1080p60, CPU/GPU/RSS/network and eight-slot policy under controlled conditions.
3. Complete Intel oneVPL/QSV native surface allocation and codec path; then platform adapters for Linux, macOS and Android with their own OS permissions and tests.
4. Only after security and consent gates, implement individually scoped input, remote monitor selection, file/audio/resource capabilities and native secure storage/live revocation. Direct connectivity/NAT traversal, signed packaging, CI, license/security review and production qualification remain open.

See [HANDOFF.md](HANDOFF.md) and [APP-VALIDATION.md](APP-VALIDATION.md) for exact completed/pending gate results rather than assuming historical roadmap milestones are current.


### Restricted decoder investigation checkpoint (2026-11-26)

Minimal strict-token worker startup, exact-LUID CUDA/CUVID capability and native headless 1080p trusted-fixture decoding now pass in Debug. Next isolate the presenter as a trusted process and qualify explicit shared GPU surfaces, real OS read/network denial (not merely token integrity) and hostile AVC input. The graphical same-executable restricted path and physical H.264 remain unqualified. Do not change the normal v8 protocol or release a remote H.264 capability until these gates pass.


### Portable media direction (2026-11-26)

Keep CPU Compact/Tiles/JPEG as the always-available media baseline on supported desktop targets. Default 1080p, selectable 720p/540p, adaptive JPEG quality; preserve 60 FPS as a target/ceiling rather than a universal guarantee. Next: add portable capability negotiation and versioned dynamic resolution/keyframe transitions, then qualify optional vendor/OS-native hardware codec backends and a CPU H.264 path independently. Do not make the isolated NVIDIA diagnostic a dependency of normal sharing. Retain consent, bounded ordered media and strict isolation gates. Platform coverage, physical network measurements and sustained multi-device 1080p60 remain open.


### Adaptive software geometry checkpoint
v10 opt-in geometry transitions and local hysteresis are implemented and locally exercised by automated protocol, encoder and authenticated QUIC loopback tests. The separate v8 fixed mode is still the default until physical v10 qualification; no fallback or native decoder access is implied. Next: locally consented physical 1080/720/540 transitions under controlled load, safe WGC scaler retargeting without new capture privileges, actual capability-based negotiation and CPU/GPU codec backend abstraction. Retain vendor-independent baseline and restricted NVIDIA security gates.


### Negotiated software and scaler retargeting checkpoint

Opt-in v11 software codec intersection and adaptive geometry are implemented, as is local live WGC/D3D11 output retargeting. The CPU codec baseline remains mandatory; no native H.264 is exposed remotely. Next qualify physical v11 sessions and multiple independent hosts under controlled pressure, add receiver-side resolution/FPS bounds, implement independently qualified platform hardware codecs and a portable software H.264 option, and complete native decoder isolation before advertising it. Preserve explicit security/backend policies; no implicit fallback from a failed strict decoder.


### Bounded viewer profile checkpoint

v12 opt-in profile intersection adds receiver height/FPS ceilings and independently configured workspace slots while retaining adaptive geometry and CPU baseline. Next validate physical v12 multi-device performance and recovery, introduce explicit measured platform capability selection, qualify independent hardware codecs and software alternatives, and complete decoder isolation. No mandatory GPU or CUDA; previous versions remain supported.


### Runtime capability checkpoint

Opt-in real CPU decoder calibration and local vendor-neutral Windows D3D11 capability inventory are implemented. Distinguish driver advertisement from actual verified conversion and from qualified hardware codec execution. Concurrent receiver cohort throughput is now measured using independent warmed production decoders. Next implement local Host capture/encode selection with consent and exact source/device verification, qualify physical multi-device sessions and platform-specific codec adapters/decoder containment. Keep CPU baseline and do not advertise native codecs from capability inventory alone.


### Host calibration checkpoint

Opt-in local measured capture/encode selection uses generated-window diagnostics before real display approval; physical monitor and sustained network qualification, optional native codec adapters and containment remain separate gates. See [HOST-CALIBRATION.md](HOST-CALIBRATION.md).


### Portable encoder optimization checkpoint

The bounded Rust JPEG writer and optional SIMD dispatch are implemented; qualification includes the independent scalar configuration and actual local A/B performance. Subsequent work should use the measured Host bottlenecks to prioritize additional CPU parallelism/encoder optimization and optional native codec adapters; physical authorized sessions remain necessary for sustained 1080p60 qualification. See [JPEG-ENCODER.md](JPEG-ENCODER.md).


### Bounded parallel software JPEG

Optional MCU-row restart strips use two or four workers on suitable CPUs, with serial compatibility and no new wire format. Benchmark serial/parallel and profile individual encode stages before attempting multi-frame capture/convert/encode overlap. Physical authorized sessions and additional native codec adapters remain separate work. See [JPEG-PARALLEL.md](JPEG-PARALLEL.md).


### Ordered local overlap checkpoint

Explicit fixed-v8/fixed-quality pipeline implemented, using two reserved local demands and one stateful CPU encoder. Adaptive transition barriers, physical authorized multi-device sessions, and optional native platform/codec qualification remain separate work. See [ORDERED-PIPELINE.md](ORDERED-PIPELINE.md).


### Capture-start scheduling checkpoint

Fixed-v8 pipeline now reserves local demands with one actual-start-clock deadline; retain opt-in fixed-quality and geometry. Measure native readiness/readback separately, receiver acceptance vs true ACK and presentation, and precise local timers only where supported without global changes. Adaptive pipeline transition barriers and physical authorized multi-device qualification remain separate gates. See [CAPTURE-GRANTS.md](CAPTURE-GRANTS.md).


### WGC capture timing qualification (2026-11-27)

WGC timing instrumentation and explicit lock experiment locally qualified at a303d03; no reproducible lock-policy speedup, so do not promote it. Next: prioritize D3D11 Map synchronization and bounded authorized staging/copy overlap (no speculative readback, no extra demand credit, preserve exact source generation, cancellation and ordered publication), plus a CUDA-independent optional absolute-deadline timer. Existing TimerWorker uses a shared response receiver; design request-specific replies and cancellation-safe shutdown before using it in a select loop. Keep Tokio portable default and compare both under identical scenes with longer runs. Separate actual ACK arrival and presentation from current local delivery. Adaptive transition barriers, physical device sessions and decoder containment remain unqualified. See WGC-CAPTURE-TIMING.md.


### Staged D3D11 readback qualification (2026-11-27)

Staged readback is implemented and locally validated at 507afc0, without a reproducible throughput gain. Retain diagnostic-only behavior. Next: implement optional CUDA-independent Windows high-resolution absolute-deadline waiting with request-specific replies and safe cancellation. Preserve portable Tokio as default; measure both capture grant and staged poll lateness independently (do not infer timer granularity from interpoll wall time alone). Prefer bounded event/readiness notifications when supported by the actual driver; compare same-scene longer ABBA including no-update, true ACK arrival, CPU/RAM and actual publication metrics. No global timer changes or busy spin. Before enabling staged Host, exercise cancel/timeout/closure/geometry change while staging is pending, verify exact generation and authorization, and establish reproducible benefit. Adaptive pipeline barriers, physical sessions and decoder containment remain pending. See STAGED-READBACK.md.

## KVM / independent capabilities priority (2026-11-27)

User prioritized public usable previews and independent multi-device keyboard/mouse control. First implement and qualify the foreground Windows input pad, including explicit and spatial focus transfer, with separate approval and transport. Then validate on two distinct owner-approved physical machines, expand to eight, measure event/ACK/application latency and recovery, qualify native local-input capture plus real physical monitor-edge topology and emergency return, and build a unified connection manager with separately authorized screen/input/resource channels and simultaneous Hybrid direction management. Clipboard, files and audio are separate future capabilities. Retain the portable CPU video path and prior timer/adaptive-media roadmap. See INPUT-SHARING.md.


### Input motion and freshness checkpoint

Local pointer consolidation and cancellation-safe bounded input acceptance are implemented. Next validate physical two-machine KVM with owner approval; do not introduce global hooks or physical edge switching before native-source/stop/release qualification. Then extend independently scoped capabilities and Hybrid sessions. Preserve the prior video timer and adaptive-pipeline work.


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

### Android manual pairing and foreground diagnostic implementation (2026-11-27)

Source extends Android preview.5 (versionCode 220005). New portable session.rs implements strict JSON requests (4096 bytes), explicit canonical peer public identity, encrypted directional grants, private numeric IPv4 routes only, local revoke, and existing intralink/probe/1 authenticated direct QUIC. BASE uses the saved outbound route; CLIENT binds its explicitly supplied private address/port and accepts one pinned BASE. No DNS route, public relay, discovery, background listener, automatic reconnect, screen or input authority. Fresh local UI approval and passphrase per diagnostic. Process-wide unique reservation before queuing the worker, stale-ID isolation, cancellation before claim/bind and cooperative cancellation during accept/exchange, explicit endpoint close and vault lock held until completion. Android Activity pause stops active/reserved diagnostic and locks on return; Stop remains available during the worker. One-time process-lifetime Iroh application JNI context installed through the documented API. Manifest requests only INTERNET and ACCESS_NETWORK_STATE. New UI saved-peer dashboard, full ID review, manual pairing and revocation.

Host gate PID 9780 exit 0 (81.12 s): strict Clippy and 3/3 actual Rust tests including encrypted persistent pairing and invalid routes/role checks, reservation isolation, real authenticated loopback QUIC and cancellation of an actual listener. Evidence benchmarks/android-pair-probe-host-20261127.txt. Kotlin and instrumentation compilation PID 23364 exit 0 (12.11 s), benchmarks/android-pair-probe-kotlin-20261127.txt; instrumentation additionally exercises pair/revoke and canceled reservation through JNI but not run on device. Native build/strict Clippy PID 1572 in progress at this source checkpoint; do not confuse host loopback with actual Android networking. New native notice archives dist/android-pairing-arm64-notices-20261127.zip and android-pairing-x86_64-notices-20261127.zip passed CRC; same package/version/license graphs as prior candidate (263 ARM64 / 264 x86_64). Gradle release runtime artifacts and POM hashes match prior exact two-package JVM archive. Cargo.lock changes only the internal Android bridge dependency list. Android validator now checks both generated and packaged JNI exports and exact two APK permissions. Existing signed/unsigned artifacts remain preserved; no release signed or published.

Hosted Android workflow 46027401383 previously failed before SDK install (sdkmanager missing, exit 127). Add pinned android-actions/setup-android v3 commit 9fc6c4e9069bf8d3d10b2204b1fb8f6ef7065407 and explicit host test step. CI correction not yet verified remotely. Next complete native ABI gates and new APK/AAB/test/lint/manifest verification, then run physical authorized diagnostic in both directions with Android/desktop, including cancellation and wrong peer. Maintain previous preview.4 desktop release unchanged. Linux X11/Wayland input adapter, signed offline invitation UI, media/input capabilities and simultaneous multi-device manager remain open. See ANDROID.md and SECURITY.md.

### Android preview.5 local candidate qualified for build/package (2026-11-27)

Native gate PID 1572 exit 0 (394.11 s): strict target Clippy and Release for both Android ABIs passed. Native libraries 9,686,504 bytes ARM64 and 11,018,424 bytes x86_64 with the actual QUIC transport. Evidence benchmarks/android-pair-probe-native-20261127.txt. Final Gradle and validator PID 6000 exit 0 (54.31 s), benchmarks/android-pair-probe-final-20261127.txt: APK Debug, instrumentation APK, unsigned Release APK/AAB, lint and runtime inventory all passed. Both generated and packaged JNI export sets exactly 10 methods, ELF architecture and 16 KiB segment/ZIP alignment checked, exact package org.intralink.android version 0.22.0-preview.5 (220005), only INTERNET and ACCESS_NETWORK_STATE permissions in main APKs. Debug/instrumentation signatures verified; release unsigned. One unsuppressed Gradle version advisory remains. Prior 3/3 host tests valid. Native and JVM license graphs unchanged and notices retained (independent provenance review still open). Candidate directory dist/android-pairing-candidate-20261127; all 10 file digests verified; older onboarding candidate preserved.

Source commits 9d32f31 and eff679b are on origin/main. Corrected hosted Android CI run 46029423784 queued at last observation; no remote pass claim. No Android device connected; physical JNI/network/cancellation/UI acceptance not performed. No signed Android public release, no video/keyboard capabilities, no new desktop source changes; published Windows/Linux preview.4 remains latest public release. Do not overwrite previous release assets. Next qualify actual Android diagnostics in both directions, then implement Linux consented input adapter and independent Android media/input capability sessions, signed offline invitations and full workspace dashboard.

- intralink-android-pairing-debug.apk: 21597815 bytes; SHA256 8f1c732b7e659bdeeffbeff544ade603e5f15b92682f4c1ebbff291c39c6d89d.
- intralink-android-pairing-instrumentation.apk: 15508 bytes; SHA256 161fb07c395901bc63cfe900e08fd7ef0f1fadf7290d9a9344f5268fddc079db.
- intralink-android-pairing-release-unsigned.aab: 9638291 bytes; SHA256 6361a8577f78e8e4d13caa1ae8abc43fe0b7d140e8734e3a1199749ae38f9ad6.
- intralink-android-pairing-release-unsigned.apk: 21380501 bytes; SHA256 900de33ba98428136a4ce8d753d8eb9e74cf943f9d531dee94373856671ab025.

### Linux native X11 receiving-host source checkpoint (2026-11-27)

Added opt-in input-x11-preview with new workspace crate intralink-input-x11 (safe XCB 1.7.1 already locked, features randr/xkb/xtest; no new external registry version). Native sink uses physical XKB names, checked XTEST 2.2 requests, RandR 1.5 single-monitor geometry, exact standard pointer mapping, ordered input state, locally held key/button down rejection, owned release with original X11 codes and nonmodifiers before modifiers. Red managed local stop window must be fully visible before listening; click/key in window, close/unmap/obscure, mapping/geometry changes or request failure stop the session. Explicit local X11 login and private IPv4, local terminal approval and independently pinned inbound grant; no background listener, privilege escalation, global capture, XWayland, SSH, public route, multi-monitor or guessed keycode fallback. X11 compatibility guards are not an OS security boundary; cleanup best effort on crash/server failure/local concurrent input. Linux CLIENT only; Windows BASE focused pad remains. Shared input_host.rs/input_command.rs extract existing host approval/transport/cleanup with separate Windows and Linux native boundary. No change to input wire format or video code.

Cross Clippy Linux x86_64 initial PID 29764 found three generated API/import errors, fixed; retry PID 30052 exit 0. Final cross Clippy after release-order improvements exit 0. Windows app strict Clippy exit 0 and 67 existing tests (58 unit + 9 CLI) passed, PID 32704 (143.53 s). New explicit/noninteractive CLI test passed separately, PID 30480 exit 0; final cross Clippy and CLI log benchmarks/linux-x11-final-cross-and-cli-20261127.txt. Preserve initial/retry cross logs and shared Windows test log. Cross check compiles adapter/test source for Linux, not native link/runtime qualification. No physical input injection performed on Windows. WSL lacks Rust, cargo and Xvfb; native hosted qualification is separate.

Manual linux-x11-candidate.yml runs strict native adapter lint/tests, ignored actual XTEST input/release/stop test only in a dedicated authenticated Xvfb display, CPU-only receiving CLI lint/tests, combined portable desktop+input-x11 feature strict lint/tests/Release/help/ldd, exact native-host Cargo inventory and TAR.GZ candidate packaging. Existing portable workflow defaults unchanged; tools accept explicit --x11-input-preview; old release tags/assets untouched. Workspace CI installs Linux XCB development libraries. No hosted X11 gate pass or public release claim yet. Android CI 46029423784 last observed SDK provisioning and host tests passed; target native build underway. Next inspect isolated Xvfb/native CI results, fix real failures, then qualify two physical Linux/Windows devices with visible stop and varied layouts before promoting. Linux BASE native focused input and Wayland compositor portal support, Android physical/MediaProjection/MediaCodec/input, capability multiplexing and physical edges remain open.

### Linux BASE source and hosted Android candidate checkpoint (2026-11-27)

Android hosted run 46029423784 completed SUCCESS for eff679b90bf37a1419daf51dbd245303acf375c8. SDK provisioning, host tests, both ABI strict native builds, Gradle APK/AAB/test, notices and validator all passed. Downloaded exact artifact into dist/android-pairing-ci-46029423784 (hosted archive digest cd349351be52770e9bcafd01834605ad3766b6a2788e116b50dad8d45ec246fd); verified all 10 SHA256 entries and ZIP/APK/AAB CRC. Candidate JSON explicitly reports debug/test signed, release unsigned, one Gradle advisory, no physical runtime qualification. Preserve local Windows-built Android candidate separately; hosted AAB size differs, not a reproducibility claim. No new Android binary source changes or public release.

Linux initial run 46031196903: native strict adapter Clippy and ordinary tests passed, actual receiving Xvfb test failed with X11 BadAtom (ChangeProperty, bad_value 0). XCB atoms_struct defaults only_if_exists=true; now explicitly create required WM_PROTOCOLS/WM_DELETE_WINDOW/_NET_WM_STATE/_NET_WM_STATE_ABOVE atoms on isolated and real servers. Added native focused XCB Linux BASE (source.rs), input_x11_ui.rs, and extracted common input_slots.rs from Windows. Both Linux host and manual workspace enabled by existing opt-in input-x11-preview; no external dependency or wire changes. XKB physical position reverse mapping and per-client detectable auto-repeat, exact window-scoped ordered input events (fast press/release retained), local reserved F1-F8 select on release, F12 local and Escape close. Neutral held-state check, one active focus, old-release/new-activate ACK and post-ACK check, latched focus/leave/visibility/resize interruption, bounded event poll, pipelined local focus/pointer/map queries. Managed startup waits for visibility before opening transport. No global hooks/grabs/root input subscription or automated physical desktop injection. No Linux spatial zones, Wayland or physical edges yet. Source native Xvfb test added; both ignored native tests run only on the dedicated isolated authenticated Xvfb server. Initial native source cross-Clippy error unused import fixed; final target cross-Clippy passed (benchmarks/linux-x11-base-cross-20261127.txt). Windows strict Clippy and 68 tests (58 unit + 10 CLI) passed after shared worker extraction, PID 21280 exit 0, 32.88 s (benchmarks/linux-x11-base-windows-20261127.txt). This is cross-source and Windows host evidence, not Linux runtime qualification. Corrected native CI pending; do not promote a release until actual native gates and candidate archive verification. No physical Android or Linux/Windows pair connected. See INPUT-SHARING.md.

### Android preview.6 local MediaProjection/AVC diagnostic checkpoint (2026-11-27)

Added non-exported CaptureActivity with generated motion fixture, fresh Android system consent, notification permission/visible Stop and focus/pause cancellation. Non-exported non-sticky CaptureService declares mediaProjection type and enters foreground before obtaining the token; one token/one VirtualDisplay. EncoderDiagnostic owns MediaCodec, input Surface, VirtualDisplay and MediaProjection callback on a single HandlerThread; bounded 15 s capture duration and setup deadline checked between native calls. Stops on local/system request, content resize/visibility change or codec failure. No reading, storing or networking captured output; only counts and timing. Main vault UI remains FLAG_SECURE. CaptureProfiles uses advertised AVC Surface encoder capabilities, portrait/landscape alignment, 1080/720/540 envelopes and configured 60/30/24 fps; hardware preferred, software allowed; not sustained FPS proof. Local diagnostic only, not remote media or control. API 26-32 compatibility keeps narrow Kotlin deprecation annotations on legacy framework consent-result/request and Parcelable accessor paths, not a Lint suppression. No new external registry/JVM dependency. Android version 0.22.0-preview.6, versionCode 220006.

Local Kotlin/Lint PID 1572 exit 0 (24.12 s). Final Gradle Debug/Release/Instrumentation/APK/AAB/runtime graph PID 24452 exit 0 (11.39 s). Actual compiled Kotlin host JVM tests PID 2368 passed 6 geometry and 8 lifecycle checks; included in CI. Instrumentation additionally compiles lease/profile assertions but not run. Final package validator PID 25008 exit 0: exact 10 JNI exports in both generated/packaged ABIs, unchanged native binary hashes ARM64 9131050ff720127ca3a97b1a05589697c52c83929a55d9db65a15a9afa9bc7f4 and x86_64 bbf4293ddeafb20000890f2f945e89233ecd9ee8ab7f0c8d5ee11a4fb2c97149, 16 KiB ELF/ZIP alignment, exact package/version, non-exported capture Activity/service and service type 0x20, exact five permissions (INTERNET, ACCESS_NETWORK_STATE, FOREGROUND_SERVICE, FOREGROUND_SERVICE_MEDIA_PROJECTION, POST_NOTIFICATIONS), debug/test signatures valid, release APK/AAB unsigned. Lint only previous unsuppressed AndroidGradlePluginVersion advisory. PID 30732 verified all ten candidate file digests and same exact registry package/version/license sets (263/264), same Maven artifact/POM hashes; old notice archives valid for unchanged graphs, independent provenance review still open. Evidence benchmarks/android-capture-kotlin-20261127.txt, android-capture-package-20261127.txt, android-capture-final-validation-20261127.txt.

Candidate directory dist/android-capture-candidate-20261127; retain prior preview.5 local and hosted candidates. No connected Android device/AVD; actual OS projection, codec output, stop/rotation/visibility/resource cleanup and cross-device network runtime not exercised. New hosted Android CI is to be dispatched for this source. Existing public desktop preview.4 unchanged. Linux native receiving test passed in 46033111814; source still failed because previous automated edit left unconditional mapping closure in source.rs. Commit 3f01ac0 replaces exact branch and passes target Clippy; corrected run 46033717616 queued at last observation. No physical KVM or release promotion claim.

- intralink-android-capture-debug.apk: 21616463 bytes; SHA256 8ffa9b953a700e9c71b0ff8ee070bcde9f1e786a3ab050ec196d7546db430ae9.
- intralink-android-capture-instrumentation.apk: 16729 bytes; SHA256 18ee543f02634f4b80467f8227303b65b756708c96c9aa7b9fb07cdc8fb1e332.
- intralink-android-capture-release-unsigned.aab: 9656665 bytes; SHA256 935a899dcf324bf885f2555de02e6d03f33a4ca3fc7299e1239a07668a561b1a.
- intralink-android-capture-release-unsigned.apk: 21415541 bytes; SHA256 a3defec86f03b7c6c516d433052db66d11b3b11a66b55b76a9c88595baf4d072.

## Current AVC producer and packaging work (2026-11-27)

Shared Rust AVC normalization and authenticated diagnostic loopback are implemented and tested; Android capture/receiver integration and independent screen consent remain open. Independent desktop candidate hash/source-doc verification is implemented and tested. Next gates remain real Android format/capture/probe, safe decoder isolation, explicit capabilities, two physical KVM machines, Wayland, distribution signing and independent provenance/security review.
