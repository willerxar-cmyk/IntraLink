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
