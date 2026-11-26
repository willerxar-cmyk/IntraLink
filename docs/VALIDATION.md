# Validation — v0.10.0

Authoring environment: no `cargo`, `rustc`, `rustfmt` or dependency cache discovered. The Rust distribution hostname could not be resolved. No Rust compilation, formatting, Clippy, Rust test execution, native permission test, QR scan, DNS update or live network probe was possible. No Cargo.lock or binary is supplied. The first build may require corrections; static file and package inspection is not a compiler, integration test or security audit.

## Test source included

- Core role directionality, exact probe protocol, DuckDNS label validation.
- Transport authenticated loopback, wrong host pin, unapproved controller, multiple candidate routes.
- Vault role isolation, revocation, self-pairing and invalid routes; authenticated round-trip, incorrect password, tampering, truncation and randomized encryption; exclusive locking, durable identity and persisted revocation.
- Signed code tampering, wrong code kind, wrong signing key, expiration, future timestamps and input bound; target mismatch, host confirmation, cancellation, replacement and replay after revocation; additive legacy schema and persisted enrollment/replay state across vault reopen.
- Bounded unique dual-stack candidates, RFC 6598 range boundaries, LAN private IPv4 hint filtering, DDNS address-mode validation.

## Execution and release gates

1. Resolve the pinned dependencies, review licenses and dependency graph, commit the actual Cargo.lock. Run `cargo fmt --all`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace --release`. Fix diagnostics without suppression. The existing CI matrix is retained and may initially require formatting/API fixes. Run on Windows, Linux, macOS.
2. Run the README manual and signed two-profile enrollment on disposable test vaults. Verify stable identities and v0.2 migration using a vault created by the prior executable. Confirm refusal of incorrect passphrases, corrupted vaults, duplicate creation, concurrent writes and unsafe paths. Exercise crash recovery, native permissions and backups. Verify that both local approvals precede any remote authorization.
3. Exercise expiration at boundaries, wrong target, wrong signer, modified route/nonce, canceled/replaced offer, replay after local revocation, clock skew and rollback. Test scan/read of terminal QR with an external scanner; malformed codes and oversized input must be rejected. Verify persistence across process restarts. Test wrong host/controller keys and invalid/oversized protocol messages. Run fuzzing and hostile-peer tests. No secrets in logs.
4. Verify direct-only traffic with packet capture in an owner-controlled network. Test opt-in mDNS registration, ID matching, stale/malicious TXT, candidate cap, cleanup on exit and privacy across interfaces. Verify no mDNS by default and no unexpected relay/directory traffic. Test DNS failure, route changes, retry bounds, IPv4/IPv6-only networks, timeout, and firewall/CGNAT scenarios. `diagnose` is classification, not NAT traversal. Do not expose the preview publicly.
5. Test DuckDNS on an owner-created test record with explicit v4/v6 and dual-stack; verify 60–3600s foreground refresh and Ctrl+C behavior, invalid token, timeouts, no proxy/redirect, bounded response and sanitized errors. Check provider DNS visibility and cache delay. Do not distribute account-wide tokens. No external endpoint has been called by the authoring process for these tests.
6. Native key protection, live revocation, audited session authorization, visible disconnect and session history, dependency review and independent security audit are required before privileged operations. NAT signaling and optional personal relay, native GUI, media and mobile/32-bit builds remain undelivered. No performance measurements are claimed.

## v0.4 historical gates (desktop v1 replaced by v2)

- Protocol record boundaries and exact view request; approved loopback frame/FIN, unapproved identity before capture callback, local source stop, diagnostic ALPN separation; JPEG fixture round-trip and malformed/oversized/wrong-dimension rejection. These are test definitions only.
- Native dependencies and optional feature CI checks are defined; run `cargo clippy -p intralink --features desktop-preview --all-targets -- -D warnings`, `cargo test -p intralink --features desktop-preview` and `cargo build -p intralink --features desktop-preview --release` on Windows and Linux. Run the original workspace gates too. No Cargo.lock, build, native screenshot, screenshot transmission or Rust test result exists in this package.
- Execute the manual and hostile-peer matrix in [DESKTOP.md](DESKTOP.md); test headless refusal, monitor disappearance, lock/sleep, user stop, malformed JPEG/headers, rate limits, deadline, loss and native OS behavior. Do not mark capture, decoder security, or 32-bit/mobile support verified from source inspection.

## Added in v0.5 (source only)

- Config round-trip/invalid dimensions, FPS, quality, duration/reserved bytes; frame and sequence bounds, exact request, repeat before first JPEG, wrong ACK, identity gate, diagnostic ALPN isolation, loopback changed/repeat/end and recycled JPEG capacity.
- Codec tests exercise real JPEG round-trip, black letterbox, exact unchanged-frame detection, one changed pixel triggering encoding, decode buffer reuse and malformed/oversized/wrong-dimension rejection. Test fixtures are not native capture substitutes.
- New transport task and single latest-frame mailbox require native tests for close/Escape during network stalls, initial wait, native capture and decode errors; inspect for stale-frame queue and no frames after local cancellation.
- Measure using [PERFORMANCE.md](PERFORMANCE.md). No Rust compiler, Cargo, rustfmt, dependency registry or toolchain download access is available in this authoring runtime. Only package/static checks were executed. No Cargo.lock, binaries, IntraLink session benchmark results or production-readiness claim were provided in v0.5. Existing source may require rustfmt, API and build corrections at first native execution.

## Added in v0.6

- Source tests for three-record credit exhaustion, unsolicited/duplicate/skipped ACK rejection, absolute sequence bound, delayed ACK with no fourth capture, bad ACK with a full window, and graceful final receipt drain. The previous one-record wrong-ACK test is replaced by a bounded-window test. These Rust tests remain unexecuted.
- The local FFmpeg utility was executed at 1920x1080/30 FPS/60 frames/2 configured threads for MJPEG, H.264, VP9 and AV1. All four encoded, decoded and passed resolution/frame-count checks. Three explicit hardware attempts at 540p failed (NVENC missing CUDA runtime, QSV session failure, AMF encoder absent). Raw JSON and method are in [CODECS.md](CODECS.md). These tests are not Rust, QUIC or hardware benchmarks.
- Repeat the full Rust gate, including delayed ACK, wrong ACK, missing receipts, local cancel during a full credit window or drain, and long-RTT tests. No Rust toolchain was discovered and the Rust distribution hostname failed DNS resolution. No Cargo.lock or compiled artifact.

## Added in v0.7 (source only)

- Mode parsing/round-trip and mismatch before capture, ordered changed record consumer, tile edge geometry, exact single-solid-tile layout, sequential raw/solid updates, malformed/truncated/duplicate/unordered/out-of-grid/oversized records rejected without mutation, budget abort and missing keyframe. The core synthetic microbenchmark is written in Rust; it has not been run.
- Both v4 modes must be validated with real consented desktop sessions and slow consumer, cancellation, malformed peer and high-motion tests. Fuzz tile and JPEG decoders and validate keyframe recovery.
- Current environment again has no `cargo`, `rustc` or `rustfmt` in PATH; Rust distribution DNS fails and crates registry lookup timed out. No Rust compilation, rustfmt, Clippy, unit tests, loopback, native capture or new benchmark was executed. Package/static checks only. Prior FFmpeg reports are historical, not current Rust performance.

## Added in v0.8 (source only)

- Palette cardinalities and index widths, padding and malformed indices, exact run coverage and truncation, edge geometry, malformed later entry without earlier canvas mutation, legacy-mode rejection, exact size/budget boundaries, palette/JPEG integration and a deterministic differential comparison with basic tiles.
- Run the dependency-free media harness from the repository root: `rustc --edition=2024 --test -D warnings crates/intralink-core/src/media_harness.rs -o intralink-media-tests.exe`, then `./intralink-media-tests.exe`. The CI gate precedes Cargo-dependent checks. Repeat the full workspace and desktop-feature gates afterward.
- Run the comparative Rust benchmark and record exact payload sizes, encoding/decoding cost and real session quality/latency. Wire-size examples are calculations, not measurements. No current Rust toolchain or dependency network access was found. No Rust test, benchmark, compilation, formatting or native session was executed. GPU integration remains pending.

## Added in v0.9 (source only)

- Test uniform keyframe coverage at all three resolutions, exact keyframe plus edge delta, malformed duplicate/missing/truncated/wrong-tag/trailing data without mutation or resize, mode refusal and high-entropy budget abort. Viewer integration test covers exact → JPEG → exact refresh and separate mode selection.
- Run the existing dependency-free Rust harness and full workspace and desktop CI gates. Run the new `keyframe_benchmark` in release mode and compare with the historical JPEG and tile baseline at equal visual fidelity.
- v6 is not wire-compatible with v5; test older peers fail closed and verify existing vault/probe behavior. No compilation, formatting, Clippy, runtime test, GPU integration or benchmark was executed here. Rust toolchain absent; Rust distribution and Debian mirror DNS resolution failed.

## Added in v0.10 (source only)

- Independent CI jobs allow the dependency-free core test to run even if formatting or Cargo resolution fails. Workspace/desktop/VA-API test and build steps still run after Clippy failures if a lockfile exists; a failed gate remains failed. No diagnostic suppression is added. Hosted CI is defined but has not been triggered from this environment.
- The first successful Cargo resolution must be reviewed and committed. CI generates a lockfile only if absent and preserves the actual workspace resolution as an artifact; subsequent Cargo commands use `--locked`. It does not declare dependency resolution reproducible across separate runs until one reviewed lockfile is committed.
- Six new Rust test definitions: four NV12 tests, two optional VA-API mapping/name tests. The probe must be tested on actual permitted render nodes (including multi-GPU, no GPU, inaccessible GPU, unsupported codec and configuration failure). Compare profiles with vendor tools, then perform a synthetic hardware frame round-trip before declaring support. See [HARDWARE.md](HARDWARE.md).
- The authoring runtime again has no cargo/rustc/rustfmt, no /dev/dri and no libva development metadata; official Rust compiler download failed. No Rust code has been compiled or tested here. Static package checks and an independent arithmetic reference check were executed; neither substitutes for Rust execution. The latter checked 140,608 RGB triplets, 256 grays and 100,000 synthetic 2x2 groups against BT.709, with maximum error one code value; see [the report](../benchmarks/nv12-reference.json).

## Added in v0.11 (source only)

- Three canvas tests for exact RGBA-to-RGB copying, odd black bars, reused-buffer clearing, and malformed geometry/length rejection without mutation; two dependency-free resource tests for invalid and boundary values; two optional resource tests for JSON and human-readable missing-value semantics.
- Added a three-OS CI feature job with Clippy, unit tests, release build and local resource sampling. Added standalone Rust runner with independent failure tracking; see [BUILD-VALIDATION.md](BUILD-VALIDATION.md). Neither the new CI job nor the runner was executed in the authoring environment.
- No compiler, Cargo or rustfmt found. New attempts to reach the official Rust distribution, crates index and Debian mirror failed DNS resolution or timed out. No native GPU devices are present. No lockfile or binaries fabricated. Only package/static checks performed. Historical arithmetic and FFmpeg reports remain historical.
- Check `resources` on actual Windows, Linux and macOS hosts, compare sampled values with OS monitors at matching intervals, exercise unsupported platforms and output errors, and verify no vault or network interaction. See [RESOURCES.md](RESOURCES.md). All gates remain open.

## Added in v0.12 (source only)

- Six dependency-free routing tests: mapped-address deduplication, invalid destinations and unsupported IPv6 scopes, per-family limits and idempotence, answer scan limit, WAN comparison, and address classification. The existing `media_harness.rs` now includes `network.rs` directly.
- CLI numeric route test: mapped IPv4 literal resolves without DNS, mapped unspecified destination is rejected.
- QUIC loopback test: bind a controller using a mapped IPv4 address, then perform the existing authenticated probe with canonical and mapped forms of the same host. Requires actual Rust dependencies and runtime; not executed.
- Rust compiler, Cargo and DNS access to distribution/registry hosts were unavailable during this revision. No new lockfile, build result, Clippy result, test result, latency measurement or GPU result is claimed. ZIP and manifest verification are structural checks only.

## Added in v0.13 (source only)

- Three dependency-free pacing tests cover thresholds, saturation, immediate change reset, slow profiles and intermittent changes.
- Two transport tests cover exact bounded allocation preparation and pointer reuse after ordered consumption in an authenticated loopback session.
- Combined-feature CI and local validation checks cover simultaneous native features. All Rust tests, builds, Clippy, rustfmt and native tests remain unexecuted here (no rustc or Cargo; static.rust-lang.org DNS resolution failed).
- Compare real v0.12/v0.13 sessions as specified in [PERFORMANCE.md](PERFORMANCE.md); check short-lived changes, idle latency, actual RSS, allocation rate, CPU, and consent/cancel. No throughput or hardware improvement is certified.

## Added in v0.14 (source only)

- Core test definitions: round-trip with incomplete, cancelled and completed attempts; duplicate/mismatched/out-of-order events; malformed UTF-8, control characters, oversize, truncation; maximum numeric sizes. App test definitions: durable reopen/unknown outcome; disabled logging and malformed-file fail-closed behavior; Unix symbolic-link and permission refusal; command error propagation; full/busy journal refusal.
- Native manual tests: two concurrent commands, Windows ACL and sharing modes, a full journal, process kill before/after the start record, partial/disk-full writes, actual completed diagnostic and GUI cancel, history after vault deletion. Verify metadata-only content, no network requests from `history`, no journal on opt-out, no changes to normal consent, and no silent truncation. See [SESSION-HISTORY.md](SESSION-HISTORY.md).
- The authoring environment still has no Rust compiler or reachable Rust distribution/registry host. No Cargo.lock or compiled binary; no Rust tests executed. The new test definitions are not test results.

## Added in v0.15 — partial Windows execution

- On an authorized Windows x64 development machine (`rustc 1.96.0`), the exact `desktop.rs`, `pacing.rs` and `workspace.rs` sources were copied to an isolated validation directory. SHA-256 matched the packaged files. A standalone public-module harness was compiled with `rustc --edition=2024 --test -D warnings harness.rs -o workspace-tests.exe` and executed: 13 passed, 0 failed. See [raw validation record](../benchmarks/workspace-windows-validation.json).
- The policy tests cover 60 FPS / session limits, version refusal, eight-record bounded credit, ordered priority, foreground/background transitions, 3 active devices, activation/deactivation and 8-slot bounds. No live video, UI or networking was involved in these tests.
- New three-host QUIC integration test is defined but unexecuted. Full Cargo workspace was not present on the Windows machine; the authoring environment has no Rust compiler. No full build, Clippy, native fullscreen, 60 FPS benchmark, GPU, multi-host runtime or mobile certification is claimed.

## Added in v0.16 — partial Windows execution

- Compiled the exact new dependency-free `presentation.rs` using `rustc --edition=2024 --test -D warnings`. Six tests passed, zero failed. The SHA-256 matches the repository source. It exercises ordered sequence bounds, geometry, pixel count, reuse of an unshared canvas, immutable snapshots across an update and invalidation on decode failure.
- Compiled and executed `tools/presentation_benchmark.rs` in optimized mode with warnings denied on the same Windows x64 machine. The exact file hashes, output and scope are recorded in [the report](../benchmarks/presentation-windows-validation.json).
- The standalone module is included in the existing dependency-free core CI harness. The full Cargo workspace, its native viewer integration, Clippy, the complete test suite, real QUIC sessions and sustained FPS remain unverified. The authoring container has no Rust compiler or registry access, and the full source archive is not present in the checked Windows project/download locations. Only the tested module and benchmark were transferred to the Windows validation folder.
- Native test: switch F1–F8 during simultaneous decode, F11 into/out of fullscreen and F12 among local monitors; fail window creation or remove a monitor, verify other sessions remain active and there is no stale cross-device frame. Measure decoder stalls and copy-on-write frequency. Do not imply remote monitor switching or GPU encoding from these changes.

## Added in v0.17 (partial Rust validation)

- Four new dependency-free geometry/thumbnail tests: all 1–8 slot layouts and nonoverlap, action separation, aspect-ratio pointer mapping including black bars/nonfinite coordinates, thumbnail aspect preservation, invalid input atomicity and no writes outside the target card.
- Two new renderer tests: live-to-closing/inactive transitions clear previous pixels, and the same allocation is reused across panel redraws at all supported counts. Six prior presentation tests reran. All 12 passed on Windows using `rustc --edition=2024 --test -D warnings` with the three actual source files (SHA-256 verified); see [overview-windows-validation.json](../benchmarks/overview-windows-validation.json).
- The full standalone `media_harness.rs` now includes the actual dependency-free overview renderer, using a self crate alias to match its normal core imports. The CI workflow includes this harness; the complete harness and the full Cargo/native integration have not been executed this turn. No committed lockfile, full build, Clippy, hardware or end-to-end performance claim.
- The complete ZIP was not found on the Windows development machine; only the tested isolated files/harness were copied into `D:\IntraLink-validation-v017`. To run all gates, transfer the complete archive to that machine and extract it into a new dedicated development directory; preserve any existing work and run [BUILD-VALIDATION.md](BUILD-VALIDATION.md). Authoring container still lacks Cargo/rustc.


## Added in v0.18 — partial Windows execution

- Four new telemetry tests validate stage separation, actual elapsed sampling, reset behavior and concurrent accounting. One new renderer regression verifies a retained thumbnail, explicit invalidation and same-sequence restart. In total 39 selected tests ran and passed using Rust 1.96.0, edition 2024 and `-D warnings`. The test sources match the packaged files by SHA-256. See [workspace-telemetry-windows-validation.json](../benchmarks/workspace-telemetry-windows-validation.json).
- This isolated harness tests portable modules and the local renderer; it does not include the entire media harness, external dependencies, native GUI or live QUIC transport. No full build, lockfile, GPU, 60 FPS or native UI claim. The complete archive was not found in the checked Windows locations; the authoring runtime lacks rustc/Cargo.
- Follow the live multi-device acceptance gates in [TELEMETRY.md](TELEMETRY.md) and the full workspace validation in [BUILD-VALIDATION.md](BUILD-VALIDATION.md).

## Added in v0.19 — portable core validation

The complete registry-independent harness and isolated Cargo core crate now each pass 65 tests (overlapping suites), with Clippy warnings denied and optimized example builds executed. Exact module hashes were compared to the package. The first Clippy finding was fixed rather than suppressed. A new regression protects the exact row-comparison optimization. See [CORE-VALIDATION.md](CORE-VALIDATION.md) and [the validation record](../benchmarks/core-windows-validation.json). Neither the full application workspace nor native live sessions have been built or tested. The separate-run codec timings were confounded; only the same-process paired microbenchmark is used for the optimization comparison.

## Added in v0.20 — actual encrypted vault validation

Eleven actual vault unit tests passed in a Windows Cargo workspace with the original core and updated vault source. The isolated Clippy gate passed with warnings denied. A subsequent joint run passed 65 core and 11 vault tests. Two new tests cover preservation of ciphertext on failed saves. See [VAULT-VALIDATION.md](VAULT-VALIDATION.md). The transport crate, complete native app, full workspace lockfile and end-to-end sessions have not been validated in this revision.


## Added in v0.21 — actual QUIC transport validation

The isolated Windows workspace includes the actual core, vault and transport crates. Eight old async test lifetime errors were corrected; the original transport tests and new multi-host test pass using real Iroh loopback transport. The subsequent isolated Cargo workspace run passed 93 tests (65 core, 11 vault, 16 transport unit and one transport integration), with Clippy warnings denied. The integration exercises signed invites and exact Compact reconstruction across three independently authorized synthetic hosts using one controller endpoint. See [TRANSPORT-VALIDATION.md](TRANSPORT-VALIDATION.md). Native UI, screen capture, physical-device networks, full lockfile, hardware video and sustained FPS remain unverified.
