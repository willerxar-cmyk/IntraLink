# Changelog

## Unreleased - Android preview.12 portable motion

- Adapt dense-motion JPEG quality using measured encoded payload and existing receipt pressure, with bounded recovery and a soft nominal target. Preserve exact lossless eligibility for sparse UI.
- Refresh the complete Compact base when returning from a lossy epoch to sparse motion; reuse the Windows JPEG output buffer without exposing failed decodes.
- Add deterministic physical generated-motion A/B and bounded offline native AVC admission diagnostics. Retain strict v9 validation; no new remote AVC/input authorization or permission.

## Unreleased - Android preview.11 entire-display navigation

- Add a separate explicit 90-second entire-display read-only sharing scope, preserving the 15-second default preview and requiring fresh local system consent. Bound navigation by authenticated capture demand, visible notification, screen-lock/task/rotation and service lifecycle checks.
- Add an opt-in standalone offline navigation test APK and real owner-assisted Android-to-Windows video/WebView/Calculator acceptance with per-phase counters.
- Retain the portable Compact/JPEG path, unchanged permissions/dependencies and unsigned distribution candidates. ADB test input is independent of IntraLink remote input.

## Unreleased - Android preview.10 read-only screen preview

- Implement an independently approved, foreground-only Android 12L+ MediaProjection source for the existing v12 Compact/JPEG viewer, with bounded direct RGBA buffers, reusable Rust canvases, peer identity recheck and local cancellation.
- Qualify actual physical Surface/ImageReader, JNI/Compact/QUIC integration, local JPEG colors and MediaTek 1080p60 configured encoder on generated pixels. Real MediaProjection and graphical presentation still require operator acceptance.
- Retain exact normal APK/AAB candidates and unchanged permissions/dependencies; no remote input or release signing.

## Unreleased - physical Android/Windows LAN acceptance

- Add owner-selected ADB instrumentation and a Windows peer example for actual private-LAN JNI/QUIC probes with fresh identities, temporary encrypted vault, revocation and lease cleanup.
- Add explicit non-distributable lan-fixture feature for generated 1080p Compact v12 traffic and complete RGB reconstruction across physical devices. Disable Release variants in fixture builds and reject fixture exports in distribution packages.
- Qualify physical ARM64 native software decode and two real 60-frame Android-to-Windows media sessions. Preserve ordinary preview.9 APK, previous candidates and documented reverse-direction UDP blocker.

## Unreleased - AVC negotiation and preview.9 native decoder qualification

- Add canonical bounded exact operating-point negotiation over pinned-identity Iroh QUIC, with independent confirmation, explicit rejection and separate loopback-only ALPN. No capture or decoder authorization.
- Add a one-shot local MediaCodec sync-frame request after a complete observed reference chain, with bounded post-request IDR observation. No remote input or media exposure.
- Qualify an actual Android software AVC decoder in instrumentation against the preserved generated fixture, comparing recovered planar YUV hashes and sampled colors. Keep fixture and test harness out of runtime APK/AAB.
- Restrict the optional Linux CI FFmpeg package installation to the signed Ubuntu archive, avoiding unrelated third-party repository failure.

## Unreleased - bounded AVC producer and preview.8 format observations

- Add two-picture bounded Rust AVC producer with loss/age/GOP recovery, fresh validated IDR, real PTS ordering and bounded keyframe requests. Reuse existing strict diagnostic packetizer and QUIC demand/receipt path; no wire change.
- Add local exact operating-point selection with independent endpoint ceilings and bounded SPS prefix observation. Verify original and recovered generated AVC with a CPU-only external development decoder, including exact decoded bytes and color samples.
- Observe actual Android MediaFormat/CSD SPS geometry/profile and PTS intervals through bounded JNI. No screen recording, remote transmission or decoder authority. Add native and emulator checks for these observations.

## Unreleased - Android preview.7 direct AVC observation

- Borrow bounded direct MediaCodec output buffers synchronously through JNI to the shared Rust NAL framing parser; retain numeric metadata only, no captured bytes or network transmission.
- Report actual observed picture-buffer rate, IDRs, config, partial/unparsed buffers, key flags and PTS separately from configured FPS. Preserve local system consent and mandatory cleanup.
- Execute actual API 36 x86_64 emulator JNI and local screen encoder tests. Add dedicated-emulator consent acceptance tool; physical runtime and public distribution remain unqualified.

## Unreleased - bounded AVC producer and candidate verification

- Add shared Rust AVC Annex B/length-prefixed NAL packetization with immutable parameter sets, AUD/IDR normalization and existing v9 structural checks; use one reusable record buffer and the authenticated diagnostic demand/receipt path. No remote Android video capability or new wire protocol.
- Verify six preserved native generated AVC access units for exact NAL payload fidelity. Add independent desktop candidate archive, source documentation, SHA-256 and feature inventory verification to candidate CI.
- Linux X11 corrected native Xvfb and combined candidate CI passed; Android preview.6 build/package CI passed. Both candidates downloaded and verified; physical and signing gates remain open.

## Unreleased - Android local capture diagnostic

- Add non-exported local moving-fixture Activity and non-sticky mediaProjection foreground service, with explicit OS consent, visible notification/Stop and bounded worker cleanup.
- Select real AVC Surface encoder capabilities, hardware optional, with aligned 1080/720/540 profiles and configured 60/30/24 fps. Release encoded buffers immediately; local metrics only, no network stream or stored images.
- Add exact manifest checks and compiled Kotlin host geometry/lifecycle tests; advance Android candidate versionCode to 220006. No physical Android encoder qualification claim.

## Unreleased - Linux X11 focused BASE

- Add native focused XCB controller pad with physical XKB events and per-client detectable repeat, preserving ordered short taps and mouse transitions.
- Share the existing bounded QUIC slot workers, target validation and vault lock with the Windows controller. Require neutral local input, release/activate ACK and post-ACK window checks; disconnect on observed focus/boundary loss.
- Correct X11 window-manager atom creation and test native input receiving and source boundaries in isolated Xvfb. Linux manual F1-F8 selection only; no global hooks, Wayland or physical screen-edge claim.

## Unreleased - native Linux X11 input host

- Add opt-in XCB/XTEST receiving adapter with XKB physical positions, single-monitor RandR geometry, checked input requests, visible local stop and owned-state cleanup.
- Share the approved input host transport and CLI lifecycle with Windows, leaving the Windows focused controller pad unchanged.
- Add native Linux isolated Xvfb and CLI qualification workflow; no Wayland or Linux controller support claim.

## Unreleased - Android diagnostic pairing

- Add explicit Android directional pairing, saved peer dashboard, revocation and one-shot private IPv4 QUIC diagnostic using the existing pinned-identity transport.
- Add process-wide single-use reservations, activity-pause cancellation and local Stop. No screen or input authorization, public routing, automatic discovery or reconnect.
- Extend host-side real QUIC tests and JNI instrumentation; enforce exact APK permissions and packaged JNI exports. Repair Android CI SDK provisioning.

## Unreleased - Android local onboarding

- Add Kotlin Android app with the shared Rust encrypted identity, role and naming operations, ABI-specific JNI builds and explicit no-backup policy.
- Add real JNI instrumentation test, localization resources, safe insets, optional authorized signing and package validation.
- Add Android candidate CI, native/JVM dependency notices and separate signed debug/unsigned release candidates. No Android remote transport or production release claim.

## 0.22.0-preview.3 - Windows input queue and native regression

- Bound queued input age and coalesce only replaceable unsent pointer motion; preserve held-key, drag, wheel, focus and acknowledgment barriers. Cancel pending sink acceptance when authenticated transport closes.
- Pass six native video regression gates on the preserved exact portable binary; retain prior automated test evidence. No physical KVM, Linux/Android distribution or production certification claim.
- Document BASE/CLIENT terminology, independent permissions and platform status.

## 0.22.0 — shared endpoint and full application validation

- Refine fixed-v8 overlap scheduling with single-use monotonic capture-start grants, early bounded demand reservations, a single foreground deadline gate and per-stage local timing. Preserve credit, consent and pixel integrity checks.

- Replace the visible high-entropy alternating pipeline diagnostic with a calm labeled scene; add independent full-canvas captured/decoded RGB comparison with local corruption and channel-swap regression tests. No production codec or wire change.

- Add opt-in fixed-v8 ordered capture/encode overlap, with at most two reserved local demands sharing existing credit, a one-worker stateful encoder lane and foreground-controlled publication/cancellation. Add generated dense-window QUIC loopback comparison; no wire change or speculative display readback.

- Add opt-in bounded two/four-worker JPEG encoding using standard restart markers, exact serial/parallel decoded-pixel regression checks, and local production stage timing. Preserve default serial path, 4:4:4 sampling, quality and record limits.

- Optimize the portable JPEG writer with optional AVX2 runtime dispatch, bounded output-buffer reuse and preserved 4:4:4 sampling/quantization; avoid provably infeasible duplicate Compact keyframe work. Keep the existing decoder and protocol.

- Add opt-in measured automatic Host capture/backend and output ceiling selection using a locally generated window and production software encoding. Preserve explicit backends, snapshot default, consent, identity, wire protocols and CPU baseline.

- Measure independent concurrent CPU decoder cohorts for 1-8 selected slots; share immutable fixtures, keep decoder/canvas ownership separate, and derive receiver limits from observed parallel frame/batch periods, not serial division. Preserve per-device ceilings and v12 wire.

- Add opt-in local CPU decoder calibration to recommend bounded v12 receiver profiles, including workspace slots. Add vendor-neutral exact-adapter D3D11 inventory and independent generated 1080p NV12 conversion verification; no automatic native codec enablement or wire changes.

- Add opt-in v12 bounded profile intersection with receiver height/FPS ceilings and independent workspace per-device overrides. Split version-specific handshakes from the shared transport. Preserve existing wire formats and authorization.

- Add opt-in v11 authenticated software capability selection and exact confirmation before capture; preserve v8/v10 and isolated v9. Retarget optional WGC GPU output at adaptive geometry transitions without changing capture source/device.

- Add opt-in v10 software adaptive geometry with separate ALPN, explicit approved ceiling, complete-keyframe transitions, local hysteresis, and safe ordered single-view/workspace presentation. Preserve the fixed v8 and isolated v9 modes.

- Default the Host to 1080p and adapt JPEG quality using bounded receipt-pressure hysteresis, with a local fixed-quality opt-out. Keep v8 geometry, exact tiles, and authorization unchanged. Validate the independent CPU feature set without NVIDIA codec dependencies.

- Separate the optional native presenter from decoder state; add a developer-only restricted headless worker for startup, CUDA capability and fixed native-generated 1080p decoding. No remote bitstream, permission, wire format or dependency changes.

- Shared outbound-only endpoint and independent view cancellation. Full Windows combined-feature Clippy and 114 tests passed; sysinfo 0.40.4 resolves dependency conflict. See [APP-VALIDATION.md](docs/APP-VALIDATION.md).

## 0.21.0 — authenticated transport and multi-host integration

- Fix eight existing async-test address lifetime errors; use stable borrowed slices in the new integration test as well.
- Add a three-host integration test using signed bilateral invitation grants, one controller endpoint, actual QUIC loopback streams and real Compact encoder/decoder. Verify each host reconstructs its own exact pixels, with foreground/background priorities.
- The isolated Windows workspace passed 93 tests and Clippy with warnings denied. See [TRANSPORT-VALIDATION.md](docs/TRANSPORT-VALIDATION.md).
- Add a test-only workspace dependency on the existing vault crate. No runtime dependency, remote privilege, or protocol changes. Native GUI and end-to-end performance remain separate validation gates.

## 0.20.0 — authenticated vault updates and real cryptographic validation

- Require a successful authenticated read of the existing vault before replacing it, and reject a change of installation identity or role. Preserve ciphertext on wrong passwords, mismatched identity and corruption.
- Add two regression tests for failed saves. Run the real vault library tests and isolated core/vault Clippy gate on Windows with pinned Iroh 1.5.0 and actual cryptographic crates.
- No new dependencies, remote permissions, envelope changes or wire-format changes. The additional Argon2id verification adds local save cost; it is not in the media path.

## 0.19.0 — portable core validation and optimized solid tiles

- Validate the complete portable harness and the isolated Cargo core library on Windows. Add an edge-tile regression for uniform rows of different colors.
- Compare complete RGB rows after validating the first uniform row; preserve the v7 wire format, exact reconstruction and existing budgets. Fix the Clippy palette-selection diagnostic.
- Run optimized CPU codec microbenchmarks before and after the change. See [CORE-VALIDATION.md](docs/CORE-VALIDATION.md) for scope, evidence and remaining native gates.

## 0.18.0 — workspace observations and stable thumbnails

- Add local per-session counters for changed records, decoded frames, distinct successful native presentation submissions, media payload bytes and stage timings; sample rates from actual monotonic elapsed time. No remote telemetry.
- Reuse the last valid thumbnail on contention or unchanged sequences, invalidate on session reuse/stop, reserve a metrics footer and preserve the bounded overview canvas.
- Five new tests and 34 selected regression tests passed in a standalone Windows Rust harness. The full workspace and end-to-end performance remain unverified. No dependency, privilege or protocol changes.

## 0.17.0 — local visual device overview

- Add the pointer-selectable overview for one to eight existing workspace slots, separate selection and per-slot activation controls, adaptive/all-live and fullscreen controls, and Tab/Escape navigation. No remote input is sent.
- Render local thumbnails from immutable decoded snapshots using one lazily allocated 960x540 canvas. Refresh at a nominal 10 FPS while open plus user actions; clear on close and disconnect; preserve the existing 60 FPS ceiling.
- Six new dependency-free tests and six presentation regression tests passed on Windows with warnings denied. Full native integration and Cargo gates remain open. No new dependency, vault or protocol change.

## 0.16.0 — independent presentation and local displays

- Release the shared canvas lock before native window presentation using immutable, reference-counted snapshots. Reuse the ordered decoder canvas when no snapshot overlaps; copy-on-write only on overlap. Fail closed after a failed decode and reject invalid geometry, lengths or ordering.
- F12 cycles local fullscreen displays without changing host capture or network sessions. Preserve the existing window on a failed fullscreen/display transition.
- Six exact Rust presentation tests passed on Windows with warnings denied. Execute the standalone three-surface 1080p microbenchmark; no native rendering, network or codec throughput claim. No new dependency, remote privilege, vault or protocol change.

## 0.15.0 — multi-device workspace and 60 FPS ceiling

- Add up to eight independent authorized view-only sessions, focus, activation, adaptive/background or all-live scheduling, per-session decoding and borderless fullscreen.
- View ALPN v7, 60 FPS ceiling/default, 3600-second approved session ceiling, eight-record credit and ordered background priority. Remove the indicator's per-event sleep from the capture loop. Vault schema and diagnostic protocol unchanged.
- Thirteen exact core tests compiled and passed on Windows with warnings denied. Full build, native UI, QUIC integration and FPS measurement are pending.

## 0.14.0 — local session-attempt history

- Add an opt-in, bounded local history for existing foreground diagnostic/view commands, with begin/end records, durable writes, file locking and explicit unknown outcomes for interrupted attempts. No payloads, peer IDs, credentials, addresses or telemetry.
- Keep the encrypted vault and v6 protocol unchanged. No new workspace dependencies; the app reuses `fs2` and test-only `tempfile`.
- Add four core tests and five file/CLI-wrapper test definitions. Rust builds and tests remain unexecuted; no production or tamper-proof audit claims.

## 0.13.0 — adaptive capture and recycled receive allocations

- Add dependency-free Rust static-scene pacing, limited to the existing maximum FPS and a 100 ms idle interval for faster profiles. Reset to the active interval after a changed record.
- Reuse consumed compressed frame allocations through a nonblocking capacity-two return pool; retain strict ordering, exact bounded payload preparation and existing receipt credit. No new wire format, vault schema, privilege or dependency.
- Add five regression test definitions and combined-feature local and CI checks. Rust compiler and registry unavailable; no Rust tests, benchmarks or native sessions executed.

## 0.12.0 — direct routing reliability

- Normalize mapped IPv4 addresses consistently before selecting and binding direct routes. Deduplicate and bound IPv4/IPv6 candidates without an auxiliary hash table. Reject unsupported scoped IPv6, unspecified, multicast and broadcast destinations. Resolve numeric addresses without DNS.
- Use the same routing policy for diagnostics and desktop transport; add a same-family WAN comparison without external lookups or new permissions.
- Add six dependency-free core tests, a numeric route test and an authenticated mapped-address loopback test. Vault, dependencies and wire format unchanged. Rust builds/tests not executed in the authoring environment.

## 0.11.0 — local resource foundation and reproducible validation

- Add optional `resource-monitor` feature for foreground local aggregate CPU and RAM sampling, bounded intervals and sample counts, human-readable or JSON Lines output; invalid metrics are unavailable rather than fabricated. No remote access.
- Move letterbox copying to a checked, dependency-free safe Rust canvas module; avoid clearing pixels that will immediately be overwritten. Add regression tests for invalid geometry and reused buffers.
- Add a standalone Rust validation runner with independent exit status reporting and an explicit initial dependency-resolution flag. Add CPU/RAM feature CI gates on desktop OSes.
- Preserve v6 desktop protocol, vault and probe. No Rust compilation or native execution occurred in the authoring environment.

## 0.10.0 — native media preparation and validation separation

- Add optional Linux VA-API probe through cros-libva: enumerate real render nodes, query planned H.264/VP9/AV1 profiles and entrypoints, attempt default configuration creation and report errors explicitly. No hardware codec or protocol change.
- Add allocation-free safe Rust RGB24-to-NV12 conversion with checked strides, BT.709 limited-range fixed-point conversion, chroma averaging and negative tests. No codec integration yet.
- Separate CI formatting, dependency-free media, workspace, desktop and VA-API feature jobs; preserve real generated dependency-resolution artifacts for review and use locked checks afterward. Do not fabricate Cargo.lock.
- Preserve v6 desktop protocol, vault and probe. Rust compilation, actual GPU probing and Rust tests remain unexecuted in the authoring environment.

## 0.9.0 — exact Rust keyframes

- Introduce Compact-only full-grid lossless keyframes, strict coverage validation and a bounded 128 KiB sender attempt; negotiated JPEG alternative for high-entropy scenes.
- Preserve ordered deltas; clamp the delta budget to 32–512 KiB, skip scratch clears for full-size output and count lossless bases separately.
- Move desktop protocol to ALPN v6; v5 peers must upgrade. Preserve vault schema and probe.
- Add core and viewer test definitions and a Rust keyframe benchmark. No new crate dependencies or external runtime codec process. No Rust compilation or benchmark execution in this environment.

## 0.8.0 — compact Rust media preview

- Add exact 2–16-color packed palettes and bounded RGB runs to the absolute tile format. Use only the smallest representation; stop run counting once it cannot improve the chosen size. No new runtime dependencies.
- Desktop ALPN v5 requires matching local codec selection; `compact` is the new CLI default. Basic tiles and JPEG remain explicit choices. No automatic downgrade. Vault and probe unchanged.
- Add dependency-free rustc test harness and CI gate, adversarial codec and differential tests, and comparative Rust microbenchmark. All new Rust test and benchmark definitions are unexecuted in this environment.

## 0.7.0 — native Rust media preview

- Added dependency-free safe Rust RGB tile codec with 32x32 blocks, raw/solid updates, bounded encoding and pre-validated application. Hybrid JPEG keyframes; no automatic codec downgrade.
- Desktop ALPN v4 negotiates matching locally selected codec mode; ordered capacity-one receiver preserves deltas; media counters and Rust synthetic benchmark added. No FFmpeg runtime.
- Added codec and transport test definitions, updated documentation; no Rust compilation or benchmark execution. Vault and probe unchanged.

## 0.6.0 — low latency preview

- Desktop ALPN v3 and fixed three-record receipt credit; ordered acknowledgment validation and bounded final drain. Upgrade both peers; diagnostic and vault formats preserved.
- New source tests for bounded delayed and invalid receipts. No new Rust dependencies.
- Standalone local synthetic FFmpeg codec benchmark, CPU encode/decode and frame-count results, and failed NVENC/QSV/AMF initialization attempts. No GPU integration or IntraLink end-to-end performance claim.
- Moved file/resource milestones behind the desktop transport and media validation gate. Rust build and tests remain unexecuted.

## 0.5.0 — desktop efficiency preview

- Replace desktop ALPN v1 with v2; host-selected 540p/720p/1080p, 1–30 FPS, JPEG quality 35–90, 60-second limit, consistent frame budget. Probe protocol and vault schema unchanged. Upgrade both desktop peers.
- Compare scaled RGB canvases exactly; skip duplicate JPEG encoding/decoding with repeat records. Reuse encoder buffers, JPEG capacity and viewer pixels.
- Demand-driven capture, receipt ACK per record, latest-frame mailbox and separate QUIC task; foreground native window remains responsive during network waits. Native capture/codec calls may still block their owning UI thread.
- Report session counters and local capture/encode/decode timings; add negative ACK/repeat tests and a benchmark plan. No new external Rust dependencies. No compilation or measured performance claimed.

## 0.4.0 — development preview

- Added optional Windows/X11 display selection, native capture via XCap, JPEG letterboxing and bounded decoding, and minimal MiniFB viewer and sharing indicator. No frame recording or input forwarding.
- Separate `intralink/view/1` ALPN with exact request, mutual pinned identity, one explicitly approved foreground session, capped frames, duration, and I/O. Existing pairing remains diagnostic-only until local consent for a view session.
- Added protocol, codec and transport test source; separate desktop-feature CI and native prerequisites. Preserved existing vault format and diagnostic path. No Rust compilation, native capture test or performance measurement has run in the authoring environment.

## 0.3.0 — development preview

- Added offline Ed25519 enrollment offers and signed controller acceptances, targeted identities, expiring single-use host confirmation and bounded controller replay state. New CLI enrollment commands and optional terminal QR output.
- Preserved v0.2 profile identity and grants with additive vault fields; older readers cannot open newly saved profiles.
- Added opt-in mDNS IPv4 discovery, fresh DNS resolution per bounded attempt, and local address classification including RFC 6598 shared IPv4.
- Added foreground DuckDNS IPv4 refresh; provider HTTP code isolated, token not persisted, redirects and proxies disabled.
- Extended interactive listener wait to 120 seconds. No relay, NAT signaling, remote enrollment endpoint, automatic privileged operations, native keystore or GUI.
- Added tests and validation instructions. No compiler or Rust runtime checks executed in authoring environment.

## 0.2.0 — development preview

- Added IntraVault with persistent Ed25519 identities, password encryption, directional peer grants, local approval and revocation, bounded authenticated profile format, staged ciphertext writes and exclusive local locking.
- Replaced ephemeral CLI `host`/`probe` with `init`, `identity`, `pair`, `devices`, `revoke`, `listen` and `connect`.
- Added bounded IPv4/IPv6 candidate selection and direct-only Iroh connection candidates. Added explicit dual-stack DDNS update option.
- Added test source and updated security, architecture and usage documentation.
- No Rust checks executed in the authoring environment. No GUI, remote control, background service or unattended agent is included.

## 0.1.0

- Initial temporary-identity direct diagnostic and one-shot DuckDNS update.

## Unreleased maintenance (v0.22.0 source)

- Consolidate the Windows source in `D:\IntraLink`, preserving the previous validation tree and locked archive.
- Correct Rust formatting and repeat the full Windows application gates.
- Add a local-only native window diagnostic for windowed and borderless fullscreen rendering, including a second local monitor when available.
- Replace 1 ms idle capture-demand polling with an asynchronous receive and 8 ms UI event deadline; preserve explicit local consent and cancellation checks.
- Preserve the v7 media protocol, vault envelope, locked dependency graph, and current remote privileges.

- Add a Windows-only native local media diagnostic with bounded samples, fixture-only capture, DPI-aware geometry/marker validation, per-stage latency percentiles, payload and throughput metrics. Preserve the production protocol, vault, dependencies and privileges.
- Record the measured local GDI-window pipeline baseline at 540p and 1080p; distinguish native source dimensions, output resolution and local sample throughput from physical remote streaming.

- Replace the desktop CPU resize path with reusable RGBA8 SIMD antialiased bilinear convolution, preserve RGB regardless of unused alpha, and avoid resizing when the source already fits. Four new regression tests passed.
- Add a Windows-only optional persistent WGC diagnostic adapter restricted to the generated child window, with bounded demand and explicit session shutdown; no new remote permissions or host capture backend changes.
- Add a CI gate for the optional WGC feature and document measured CPU stage improvements and limitations. Resolve the pinned dependencies and keep existing dependency versions and protocol v7 unchanged.

- Add the isolated optional `intralink-d3d11` native adapter with documented unsafe COM calls, checked RGBA8 texture/mapping geometry, retained staging resource and automatic unmap. Recycle the native RGBA allocation through the bounded WGC diagnostic channel; the application encoder borrows the input.
- Add explicit strict `--gpu-scale` WGC diagnostic mode using the D3D11 video processor, retained input/output surfaces, queried SDR RGB format support and pre-readback aspect-preserving scaling. No CPU fallback on driver errors. No new external package versions or production sharing permissions.

- Retain the most recent native WGC frame on the GPU independently of consumer demand. Coalesce notification wakes, track observed generations, eliminate a redundant video-processor input surface and preserve the final frame after idle changes. CPU readback remains demand-driven.
- Add bounded cancellable waiting and separate no-new-sample reporting; do not treat absent native events as encoded repeat frames. Add a two-state static fixture and native idle/final-update diagnostic. No new external dependencies, network protocol changes or remote privileges.

- Replace the static diagnostic's thin midpoint marker with two interior marker positions and exact checks of both the current marker and restored background. Add scaled positive and stale-frame negative assertions and contextual native errors. This changes the generated fixture only, not Host capture or authorization.

### 0.22.0 development — mailbox lifecycle validation
- Strengthened static marker validation with large solid-color regions, exact state identification and scaling regression checks.
- Added explicit local cancellation and native WGC source closure checks to the fixture; verify that capture is rejected after closure and stop.
- Preserved the existing Host backend and authorization flow. Intermediate binaries are retained as unqualified artifacts.

- Final Windows verification: 121 tests, strict Clippy for both feature combinations, Rustfmt, Release build and all three native fixture diagnostics passed; physical device loss/topology and actual remote streaming remain open.

### Native D3D11 ownership and geometry guard (v0.22.0 development)
- Enable verified native immediate-context multithread protection in addition to the WGC handler lock. Check device removal before/after retention and before/after readback, and reject a context owned by a different device.
- Add a local hardware diagnostic using two independent hardware D3D11 devices, initialized textures, foreign context/texture rejection, staging identity/geometry rejection, and unchanged output on invalid inputs. Add an owned-window resize fixture with an explicit parent-to-child pipe trigger, actual WGC callback failure observation and rejection after stop.
- No new remote privileges or wire-format change. Actual GPU removal, full-display topology and the WGC Host remain separate gates.

### Explicit WGC Host and desktop wire v8 (v0.22.0 development)
- Add explicit `share-desktop --capture snapshot|wgc|wgc-gpu`. Keep snapshot as the default; unavailable native features, driver failures, ambiguous monitor IDs and changed display identity/geometry fail closed without automatic backend fallback.
- Instantiate the persistent WGC mailbox only after the existing locally approved peer and matching view request produce capture demand. Preserve foreground stop, native capture-border defaults, source geometry checks and shutdown. Add a conservative 15-second no-new-readback watchdog.
- Version the desktop ALPN/request to v8 and reserve a distinct bounded no-update record. Maintain ordered ACK credit, independent sessions, codec IDs and vault/probe compatibility. No-update is not a new frame or verified repeat; never permit it before the first changed record.
- Add a per-session no-update notice on the viewer/workspace after one second of consecutive no-update records, including fullscreen and overview. The presentation-only notice uses a reused scratch canvas and never modifies the decoder surface.
- Physical full-display WGC streaming, monitor loss, suspend/resume, actual GPU removal and sustained network performance remain separate acceptance gates.

### Native GPU color conversion (development)
- Share D3D11 video processor logic between RGBA scaling and GPU-resident NV12 BT.709 limited-range conversion; add bounded planar staging layout and borrowed native output surface.
- Add real generated-color GPU round-trip diagnostic and explicit DXGI vendor/index selection with no software fallback, ambiguity or vendor mismatch.
- Preserve v8 desktop transport and existing Compact/Tiles/JPEG codec behavior. Hardware video encoding/decoding remains pending.

### Native NVENC H.264 preview (development)
- Add the optional Windows nvenc-preview feature with fixed SDK 13.1 raw ABI bindings, System32-only driver loading, strict NVIDIA adapter/texture validation, low-latency H.264 configuration and bounded synchronous output.
- Register the retained NV12 texture once; validate sequence, requested IDR/SPS/PPS and output size; fail closed on driver errors. Add generated-frame diagnostic with optional bounded clip export.
- No changes to view v8, Host capture authorization, existing codecs, or default feature policy. Native decoder and production qualification remain pending.

- Add a native NVIDIA CUDA/CUVID decoder diagnostic using the same adapter LUID, a private context, bounded H.264 sequence and picture callbacks, native decode status and sampled NV12 color validation. Explicitly record driver hardware status values and preserve independent decoder evidence. No remote bitstream decoder is exposed yet.

### Isolated H.264 transport v9 (development)
- Add a bounded H.264 profile and versioned access-unit envelope, strict sequence/IDR/parameter-set structure and coded GOP policy.
- Share the existing v8 capture-demand/receipt engine with a separate, loopback-only v9 ALPN and exact profile offer/echo. Reject unauthorized identities or profiles before capture, malformed access units before delivery/acknowledgment, and keep coded/receipt sequences separate.
- Add native fixture-to-QUIC-to-NVDEC diagnostic with exact generated-byte matching before driver submission; no live capture, new remote privileges or production H.264 advertisement.
- Preserve previous v8 formats, codec choices, vault and probe. Native decoder safety for arbitrary input remains a separate gate.

- Fix Windows Debug CLI entry-point stack overflow by parsing synchronously and constructing only the selected async command future. Add process-level CLI smoke tests.
- Add optional private high-resolution waitable timer diagnostic using one native handle and one bounded joined worker; reject early/failed wait before another capture. No global timer-resolution setting, busy spin or change to v8 codec selection.

### Selected-adapter WGC codec pipeline (development)
- Create an explicitly selected NVIDIA D3D11-backed free-threaded WGC pool for only the generated child window; retain stable RGBA/NV12 GPU surfaces and encode/decode directly on the selected adapter. Validate geometry, timestamps, device ownership, source closure and cleanup.
- Add local GPU presentation development using CUDA/D3D11 plane textures and a dedicated visible DXGI swap-chain window. Verify sampled colors; no arbitrary remote AVC or new remote permissions. Normal Host remains v8.

### Restricted H.264 profile and simultaneous local GPU transport
- Add bounded SPS/PPS parsing and exact level/geometry/SDR/FPS checks to the isolated v9 profile; correct RBSP escape fixtures and strict lint.
- Reuse selected NVIDIA WGC encoder and native NVDEC/DXGI paths in independent thread-owned continuous loopback workers with bounded demand, origin tokens and ordered native submission receipts. Verify exact locally generated packet bytes before native decode; no arbitrary remote AVC.
- Add native cancellation/teardown gate, per-run source and steady native-submit timing, exact wire-origin regression and CLI checks. Preserve normal v8 behavior. Archive the unlinked preliminary source draft outside the active workspace.

### Supervised decoder process (development)
- Add explicit --isolated-decoder diagnostic using a separate owned executable, private bounded IPC and a Windows Job Object with kill-on-close and one-process limit. Keep current local generated-origin verification and v9 validation on both sides.
- Require immediate ordered native display callbacks before receipt, reject invalid bootstrap/record/response limits, and supervise actual child exit and cancellation. No arbitrary remote AVC or physical sharing exposure.

### Decoder process hardening (development)
- Enforce 2 GiB committed-memory limit and unhandled-exception policy in the decoder job; verify image-load, extension-point and child-creation mitigations before native GPU initialization.
- Add explicit experimental low-integrity primary-token launcher with exact handle allowlist, minimal environment, job before resume and real token-only validation. Restricted graphical startup remains unqualified (0xC0000142); no fallback.
- Preserve child native exit diagnostics, verify mandatory no-write-up, exact restricted SID membership and unchanged parent token. Add CLI guard coverage and native evidence.

## 0.22.0-preview.4 - onboarding and role migration

- Add a local BASE/CLIENT graphical first-run selector for Windows/Linux desktop builds, terminal-only fallback, encrypted device names and authenticated role changes.
- Preserve identity, compatible directional grants and historical invitation replay receipts; explicitly revoke incompatible grants and cancel outstanding offers during migration. Existing ordinary vault save guards remain.
- Repair exact dependency notice discovery for Linux, including nested LICENSES and REUSE metadata, repository subdirectory URLs, build-host metadata and documented notice provenance exceptions.
- Pass the six Windows native diagnostic gates; produce and verify a Linux x86_64 native CI candidate archive.
