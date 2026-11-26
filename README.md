# IntraLink

**Remote access, without mandatory central servers.**

Version 0.22.0 — development preview. The complete locked Windows workspace has passed automated tests and strict Clippy. A real loopback integration test reconstructs independent Compact images from three paired hosts through one controller endpoint. Native generated-window WGC/D3D11 diagnostics have executed; authorized physical full-display sharing, physical multi-device sessions, hardware codecs and sustained end-to-end 60 FPS remain unvalidated. See [current validation](docs/APP-VALIDATION.md) rather than treating historical source milestones below as release certification.

### v0.22 additions

- Portable media baseline: `desktop-preview` supports CPU Compact/Tiles/JPEG without CUDA. Normal Host defaults to 1080p, with selectable 720p/540p and bounded adaptive JPEG quality. `--fixed-quality` disables adaptation; hardware acceleration remains optional. See [DESKTOP.md](docs/DESKTOP.md).

- Restricted headless NVDEC development diagnostic: the minimal worker starts under the existing reduced token, matches the exact CUDA LUID and decodes a fixed locally generated 1080p fixture. No physical H.264 sharing, complete OS sandbox or shared GPU surface transport yet. See [SECURITY.md](docs/SECURITY.md) and [APP-VALIDATION.md](docs/APP-VALIDATION.md).

- Shared workspace endpoint, independent cancellation, full Windows application Clippy and 114 tests passed. See [APP-VALIDATION.md](docs/APP-VALIDATION.md).
- Windows maintenance: locked Release build, formatting and repeated test gates passed. `intralink desktop-check` passed local windowed and primary 4K fullscreen buffer submission; it does not capture or transmit the desktop. Host capture demand now wakes asynchronously. See [DESKTOP.md](docs/DESKTOP.md).
- Windows native local media diagnostic: `intralink desktop-media-check --height 1080 --samples 30 --codec compact` exercises isolated window capture, CPU codecs and local native buffer submission. See [PERFORMANCE.md](docs/PERFORMANCE.md) for measured bottlenecks and limits.
- SIMD CPU resize: bounded reusable RGBA8 buffer, antialiased bilinear filtering, exact-size fast path and 119 workspace tests passed. The optional Windows-only `wgc-preview` feature adds persistent-capture diagnostics (`desktop-media-check --wgc`). See [HARDWARE.md](docs/HARDWARE.md).
- Reusable native D3D11 staging texture and recycled RGBA buffer; `desktop-media-check --wgc --gpu-scale` additionally requests strict hardware video-processor scaling before readback. See [PERFORMANCE.md](docs/PERFORMANCE.md) for actual local measurements.

- The latest development source adds explicit Windows Host capture selection (`share-desktop --capture snapshot|wgc|wgc-gpu`), desktop wire v8 no-update records and local last-frame notices. Both peers need v8; actual authorized full-display operation remains unvalidated. See [DESKTOP.md](docs/DESKTOP.md) and [APP-VALIDATION.md](docs/APP-VALIDATION.md).

- GPU video preparation: native D3D11 RGBA-to-NV12 BT.709 limited-range conversion, retained GPU surface, strict adapter selection, and real color round-trips on NVIDIA and Intel. Hardware encode/decode is still a separate integration gate; see [HARDWARE.md](docs/HARDWARE.md).

### v0.21 additions

- Fix eight existing asynchronous test lifetime errors and avoid temporary address slices in the new integration test.
- Validate the three-library workspace (93 tests passed; Clippy passed) and add a real, synthetic three-host test linking signed invitations, pinned QUIC connections and independent Compact decoders.
- Keep the wire format, production dependencies and remote privileges unchanged. See [transport validation](docs/TRANSPORT-VALIDATION.md).

### v0.20 additions

- Authenticate the existing encrypted vault before every normal save, and preserve its identity and role. Wrong passwords and corrupted ciphertext must not be overwritten. Explicit password change and key rotation are not yet implemented.
- Test the actual vault crate on Windows with real cryptographic dependencies: 11 tests passed, including two new storage regressions. The isolated core/vault Clippy gate passed. See [vault validation](docs/VAULT-VALIDATION.md).
- Preserve the existing encryption envelope, grants, wire protocol and dependency declarations. The isolated validation workspace lockfile is not a lockfile for the full application.

### v0.19 additions

- Compare an initially uniform RGB row with the remaining tile rows using exact slice equality. Preserve the binary format and lossless reconstruction; no new dependencies or unsafe code.
- Fix the reported Clippy diagnostic in Compact palette selection without suppressing warnings.
- See [the core validation and benchmark results](docs/CORE-VALIDATION.md). The Windows validation workspace is deliberately isolated from other projects and does not contain all application crates; its lockfile is not a complete project lockfile.

### v0.18 additions

- Sample changed-frame rates and media payload throughput separately for each workspace session, without external telemetry or persistent logs. Window title and overview cards report measured local values rather than the configured FPS ceiling.
- Keep the last complete thumbnail while the decoder is busy or no newer sequence exists. Clear it on disconnect, decoder invalidation, or slot reuse; avoid repeatedly resampling identical frames.
- See [TELEMETRY.md](docs/TELEMETRY.md) and [Windows validation](benchmarks/workspace-telemetry-windows-validation.json). No new dependencies, remote privileges, vault format or wire protocol changes.

### v0.17 additions

- Tab opens the local device overview. Click a thumbnail to select an existing device session or its ON/OFF button to activate or disconnect that slot. The overview has all-live/adaptive, fullscreen and back controls. Other sessions remain independent. Escape closes the overview before exiting the workspace.
- A lazily allocated 960x540 RGB32 canvas is reused for thumbnails derived from existing decoded surfaces, refreshed at a nominal 10 FPS while visible plus interaction-triggered refreshes. No additional network requests, video compression or remote pointer forwarding. Inactive cards are cleared; thumbnails are not persisted. Full native pointer/DPI validation is pending.
- No new dependency, protocol, vault format or remote privileges. See [WORKSPACE.md](docs/WORKSPACE.md), [VALIDATION.md](docs/VALIDATION.md) and the [Windows harness results](benchmarks/overview-windows-validation.json).

### v0.16 additions

- Immutable presentation snapshots keep the native window outside the shared decoder lock. The ordered canvas is reused unless a transient presentation reference overlaps a decode; in that case safe Rust copy-on-write preserves the previously displayed image.
- F12 moves the fullscreen viewer to the next **local** monitor. A failed fullscreen/display transition keeps the previous window and all active sessions. This is not remote monitor switching.
- Six new core tests executed on Windows with warnings denied. Three-canvas 1080p CPU microbenchmark executed with and without forced copy-on-write; see [PERFORMANCE.md](docs/PERFORMANCE.md). No new dependency or wire-format change.

### v0.15 additions (historical)

- Workspace for up to eight paired hosts, with independent ordered decoder workers, focus switching, explicit connect/disconnect, adaptive 5 FPS background sampling or all-live scheduling, and borderless fullscreen. See [WORKSPACE.md](docs/WORKSPACE.md).
- View protocol v7, 60 FPS maximum/default, 300-second default and 3600-second maximum explicitly authorized session, eight-record credit, and ordered background-priority feedback. Both peers must upgrade.
- 13 actual dependency-free Rust tests passed on Windows; the full Cargo build, native session and performance gates remain open.

### v0.14 additions (historical)

- Optional local attempt history for existing `listen`, `connect`, `share-desktop` and `view-desktop` commands, enabled per invocation with `--record-session`. The `history` command displays bounded metadata, not session content or peer identities. See [SESSION-HISTORY.md](docs/SESSION-HISTORY.md).
- Durable begin/end records distinguish protocol completion, local cancellation, command errors and an unknown outcome after interruption. A full or malformed journal refuses a new recorded command before network activity. No new remote capabilities or wire-format change.
- Nine additional test definitions for strict history parsing and local file behavior; no Rust compilation or execution has occurred. The app reuses `fs2` and `tempfile` already present in the workspace.

### v0.13 additions

- Rust capture pacing: after four unchanged records, double the interval; after twelve, quadruple it, capped at 100 ms for profiles faster than 10 FPS. Changed records reset the interval. The configured FPS remains an upper bound, not a guarantee. See [PERFORMANCE.md](docs/PERFORMANCE.md).
- Recycle consumed compressed payload buffers through a nonblocking capacity-two pool, preserving ordered delivery, the capacity-one pending queue and wire v6. A missing buffer allocates normally; no zero-copy or measured performance claim.
- Add five regression test definitions and combined-feature Clippy/test/release gates for Windows and Linux. No new dependency or remote operation. The Rust toolchain is still unavailable; no compiled build or executed Rust test is claimed.

### v0.12 additions

- Shared dependency-free Rust routing policy for DNS resolution, diagnostic probes and desktop sessions. Normalize IPv4-mapped IPv6, deduplicate routes before per-family limits, reject unspecified, multicast, broadcast and unsupported interface-scoped IPv6 destinations, and bypass DNS for numeric IP addresses.
- Compare the owner-supplied WAN address with the selected same-family routes, without making public-IP requests or claiming NAT detection. See [NETWORKING.md](docs/NETWORKING.md).
- Six core routing tests plus literal-route and authenticated loopback test definitions. No protocol or vault changes; Rust execution is still pending.

### v0.11 additions

- Optional foreground **local** CPU/RAM monitor using Rust and native OS APIs through `sysinfo`. No network, vault, process enumeration, remote resource access, or background collection. See [RESOURCES.md](docs/RESOURCES.md).
- Single-pass safe Rust letterboxing that clears only black bars and preserves aspect ratio without an additional full-canvas clear.
- Standalone cross-platform Rust validation runner for the actual core and workspace tests, optional feature gates, Clippy and release builds. See [BUILD-VALIDATION.md](docs/BUILD-VALIDATION.md). All Rust builds and tests remain unexecuted in the authoring environment.

IntraLink is an independent Rust project inspired by DWService's functional organization. One application, controller/host/hybrid roles, no mandatory account, directory or relay. DNS is routing, not identity. See [architecture](docs/ARCHITECTURE.md), [security](docs/SECURITY.md), [vault format](docs/VAULT.md), [enrollment](docs/ENROLLMENT.md), [networking](docs/NETWORKING.md), [protocol](docs/PROTOCOL.md), [platforms](docs/PLATFORMS.md), [roadmap](docs/ROADMAP.md), and [validation](docs/VALIDATION.md).

## Current source

- Password-encrypted local profiles, persistent identities, manually approved directional peer grants, listing and revocation.
- Versioned authenticated encryption (Argon2id + XChaCha20-Poly1305), local file lock and same-directory staged writes. No central vault or credentials in command arguments.
- One-shot direct Iroh QUIC diagnostic; up to four IPv4 and four IPv6 DNS candidates; host and controller identities pinned independently. No default relay or address lookup.
- Offline, signed, targeted invitations with expiring offer, signed acceptance and separate final local host approval. Optional terminal QR rendering; paste a code obtained from an external scanner. One pending offer per host and bounded replay state in the encrypted vault.
- Opt-in IPv4 LAN mDNS discovery for the already paired host, followed by the saved DNS/IP route; up to three bounded connection attempts with fresh route resolution; address-class diagnostics with optional owner-provided WAN IP.
- Explicit DuckDNS update for one existing record, supporting IPv4/IPv6 and a dual-stack update. Optional periodic foreground IPv4 refresh. Account token entered interactively; never distributed automatically.
- Optional `desktop-preview` feature: locally selected display, session-specific consent, separate QUIC ALPN v7, selectable 540p/720p/1080p and 1–60 FPS target, unchanged-frame suppression, bounded eight-record receipt backpressure, Rust compact palette/run tile codec with lossless keyframes and explicitly negotiated JPEG alternative, explicit codec negotiation, bounded ordered receiver, reusable buffers, independent network task, visible local stop and metrics. See [DESKTOP.md](docs/DESKTOP.md) and [TILES.md](docs/TILES.md).
- Optional local Linux `vaapi-probe` feature: per-render-node driver profile/entrypoint and default configuration diagnostics. Safe Rust RGB24-to-NV12 conversion preparation, with bounded strides and reusable planes. Neither enables a hardware codec on the wire. See [HARDWARE.md](docs/HARDWARE.md).
- Unit and loopback test source and independent media, format, workspace, desktop and optional VA-API CI jobs. The core and vault libraries passed isolated Cargo tests and Clippy on Windows; full workspace formatting, builds and native tests remain pending.

No background listener, remote terminal, file access, remote keyboard/mouse, unattended control, automatic DNS provisioning, native secure storage, integrated QR camera scanner or full management GUI yet. The optional desktop viewer is not the planned unified Tauri interface. Use only on authorized local test devices. Do not expose this unvalidated preview to the internet.

## Multi-device workspace

After pairing and individually authorizing each Host, start the controller (example saved names):

```sh
cargo run -p intralink --features desktop-preview -- --vault main.vault workspace --device workstation --device notebook --device server --fullscreen
```

Use Tab to open the device overview, click a thumbnail to select or ON/OFF to toggle its session; F1–F8 switch directly, F9 toggles the selected slot, F10 changes all-live mode, F11 toggles fullscreen and F12 cycles local displays. Escape closes the overview first, then exits the workspace. Reopening an ended session requires fresh local Host authorization. No input forwarding or mobile viewer yet. See [WORKSPACE.md](docs/WORKSPACE.md).

## Dependency-free media test gate

With a current stable Rust compiler installed, run these from the repository root even when Cargo cannot download dependencies:

```sh
rustc --edition=2024 --test -D warnings crates/intralink-core/src/media_harness.rs -o intralink-media-tests.exe
./intralink-media-tests.exe
```

This compiles the actual media, color conversion, desktop protocol and routing source directly, without the Cargo registry. It does not compile the vault, Iroh, native viewer or other workspace crates. The standalone gate and full workspace checks are also defined in CI. See [TILES.md](docs/TILES.md) for the new default compact mode, wire layout and benchmarks.

## Build prerequisites

Install current stable Rust and native build dependencies. The first successful resolution must generate and review `Cargo.lock`. No lockfile is fabricated. Run the following from the repository root:

```sh
cargo generate-lockfile # Only if Cargo.lock does not already exist; review and commit it.
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace --release
```

## Test with two local profiles

The same executable serves both roles. If no `--vault` is provided, it uses the platform's local application data directory. Separate files below allow a loopback test on one machine. Choose a private, user-owned directory; use strong unique passphrases and save them securely. Commands below are intended after a successful build. No secret key should ever be copied to another device.

```sh
cargo run -p intralink -- --vault host.vault init --role host
cargo run -p intralink -- --vault main.vault init --role controller
```

Each command prints the **public** identity. Store both public IDs for the next commands, or retrieve each with `identity`. Replace `CONTROLLER_PUBLIC_ID` and `HOST_PUBLIC_ID` with those IDs. Verify the complete IDs over a trusted independent channel and type `APPROVE` at each prompt.

```sh
cargo run -p intralink -- --vault host.vault pair --name main --direction inbound --peer-id CONTROLLER_PUBLIC_ID
cargo run -p intralink -- --vault main.vault pair --name workstation --direction outbound --peer-id HOST_PUBLIC_ID --host 127.0.0.1 --port 45100
```

In terminal A, unlock and approve the one-shot listener, then immediately connect from terminal B (incoming wait is at most 120 seconds after the listener starts):

```sh
# Terminal A
cargo run -p intralink -- --vault host.vault listen --bind 127.0.0.1:45100 --controller main
# Terminal B
cargo run -p intralink -- --vault main.vault connect --device workstation
```

Prepare both terminals and unlock B promptly. The host must accept the controller identity and the controller must authenticate the host identity. A successful fixed probe closes the session. Neither side receives a remote shell or file/desktop permission. Ctrl+C terminates the local foreground process. There is no background service.

```sh
cargo run -p intralink -- --vault main.vault devices
cargo run -p intralink -- --vault host.vault revoke --name main
```

Revocation affects the local grant only. Close the listener first; the vault lock prevents changes while a local command has it open. Subsequent sessions using the saved grant are denied. Restoring an older vault can restore old grants, so check and revoke them again after restoring a backup.

For an authorized LAN test use the host's actual reachable address, and choose that address when binding. An existing DuckDNS name can be stored instead of an IP. DNS does not solve NAT/CGNAT or firewalls. Do not configure internet exposure until the required security review and network tests pass.

## Signed invitations and LAN discovery

As an alternative to manual `pair`, initialize the two profiles and exchange public IDs independently. On the host, issue a 5-minute invitation addressed to the controller. Copy the host code into the controller prompt, and then return the signed acceptance to the host. Each local command requires the vault passphrase and explicit approval.

```sh
# Host: replace CONTROLLER_PUBLIC_ID with the verified controller identity.
cargo run -p intralink -- --vault host.vault invite --controller-id CONTROLLER_PUBLIC_ID --host 127.0.0.1 --port 45100 --ttl 300 --qr
# Controller: paste the host code when prompted, verify the IDs and approve.
cargo run -p intralink -- --vault main.vault accept-invite --name workstation --qr
# Host: paste the controller's signed acceptance and approve.
cargo run -p intralink -- --vault host.vault confirm-invite --name main
```

Do not execute this flow if you already added the corresponding manual grants. A new invitation replaces the previous outstanding offer. `cancel-invite` cancels an outstanding offer without revoking existing grants. QR rendering is optional; there is no integrated scanner. Do not publish enrollment codes because they disclose routing and public identities. See [ENROLLMENT.md](docs/ENROLLMENT.md).

For a private LAN test only, replace `HOST_PRIVATE_IPV4` with the host's actual interface address. Both commands use the same identity as the preceding saved grant. `--advertise-lan` is explicit consent to advertise the public identity and route on the local network. `--lan` requests mDNS discovery of the paired target, not automatic trust. Use the commands in separate terminals:

```sh
cargo run -p intralink -- --vault host.vault listen --bind HOST_PRIVATE_IPV4:45100 --controller main --advertise-lan
cargo run -p intralink -- --vault main.vault connect --device workstation --lan --attempts 2
cargo run -p intralink -- --vault main.vault diagnose --device workstation
```

The host still serves exactly one approved diagnostic session and terminates afterward. If an attempt reaches a host that already closed, the host must start a new listener. Fresh DNS resolution may still return cached records. NAT type cannot be inferred conclusively from a DNS answer or a WAN IP; see [NETWORKING.md](docs/NETWORKING.md).

## Codec feasibility benchmark

The optional development utility `python3 tools/codec_benchmark.py --height 1080 --fps 30 --seconds 2 --threads 2` requires locally installed FFmpeg and FFprobe. It generates synthetic video only, encodes and verifies decoding for four CPU codec profiles. See [CODECS.md](docs/CODECS.md) for hardware checks, results and limitations. This utility is not a runtime dependency or a replacement codec for the Rust viewer.

## Native Rust media benchmark

`cargo run --release -p intralink-core --example tile_benchmark` benchmarks the platform-neutral tile codec using synthetic RGB input and verifies decoded output. This new benchmark is not yet executed. See [TILES.md](docs/TILES.md).

## Experimental desktop preview

After completing the paired-profile test above, build the optional feature on Windows or a Linux X11 desktop (native dependencies and limits in [DESKTOP.md](docs/DESKTOP.md)). On the host, first enumerate local displays and choose the numeric ID. Use a disposable test desktop with no sensitive content. Start one foreground session and explicitly approve the named controller and selected display; on the controller, start the viewer using its saved outbound route. These are intended commands, not a record of a successful execution.

```sh
cargo build -p intralink --features desktop-preview
# Host: list displays and replace DISPLAY_ID with an actual local ID.
cargo run -p intralink --features desktop-preview -- displays
cargo run -p intralink --features desktop-preview -- --vault host.vault share-desktop --bind 127.0.0.1:45100 --controller main --monitor DISPLAY_ID --seconds 30 --fps 15 --height 540 --quality 75 --codec compact
# Controller, in another terminal:
cargo run -p intralink --features desktop-preview -- --vault main.vault view-desktop --device workstation --codec compact
```

Both ends must explicitly use the same codec mode (`compact`, the default, `tiles`, or `jpeg`); the host starts capturing only after verifying the controller identity and matching view request. Close the local red sharing window, press Escape while it is focused, or terminate the host process with Ctrl+C. The viewer can close its window or press Escape. Capture stops after the local time or frame limit; native blocking calls can delay normal UI processing, so Ctrl+C remains the foreground emergency stop. No keyboard/mouse forwarding. Use only on local test networks. Do not run the diagnostic listener and viewer listener on the same port simultaneously. Screen-view mode resolves the saved DNS/IP route, without LAN discovery or automatic reconnect yet. The FPS setting is a ceiling, not a measured capability; an eight-record credit window reduces stop-and-wait while bounding the conforming-peer backlog. See [PERFORMANCE.md](docs/PERFORMANCE.md) for instrumentation and benchmark gates.

## DuckDNS

```sh
cargo run -p intralink -- ddns --domain YOUR_REGISTERED_LABEL
cargo run -p intralink -- ddns --domain YOUR_REGISTERED_LABEL --ip YOUR_PUBLIC_IPV4 --ipv6 YOUR_PUBLIC_IPV6
```

The second command uses placeholders for verified addresses. A token is prompted locally and not saved. `--ip` alone accepts either IP family, `--ipv6` alone disables IPv4 autodetection. The API does not register new names. Keep the account token with its owner or on an owner-administered router; do not share an account-wide token with all hosts. For owner-operated foreground IPv4 autodetection, optionally use `ddns --domain YOUR_REGISTERED_LABEL --interval 300`. Run it only from the network whose public IPv4 the record should contain; it stops with Ctrl+C and does not store its token. Do not run it from the controller network for an unrelated host. Explicit IPv6 and dual-stack updates remain one-shot. Additional provider implementations are future work.

## Compatibility and license

The old temporary `host` and `probe` CLI commands were intentionally replaced by `init`, `pair`, `listen` and `connect`. The probe wire protocol is unchanged. v0.3 through v0.12 accept v0.2 vault JSON with missing enrollment fields and preserve its identity and grants. Once saved by v0.3 through v0.12, do not downgrade the vault to v0.2: the earlier reader rejects unknown fields. Back up before upgrading; migration must still be verified by the test suite. The desktop ALPN changed from `intralink/view/5` to `intralink/view/6` in v0.9 and remains v6 through v0.12. The v0.10–v0.12 changes do not alter the wire format; interoperation remains untested. v0.15 introduces `intralink/view/7`; both endpoints must upgrade. The diagnostic protocol and vault schema are unchanged.

Proposed MPL-2.0. No DWService code copied. Review [references and licenses](docs/REFERENCES.md) before publishing. No platform build or runtime parity has been established.

### Current desktop compatibility

The latest development source binds `intralink/view/8` exclusively. It is not compatible with v7 desktop peers; update both endpoints. The probe protocol and encrypted vault are unchanged. Explicit WGC Host capture is optional and is not production-qualified. Use a locally approved disposable test display and a paired controller.

### Windows native NVIDIA codec diagnostics

The optional `nvenc-preview` feature builds native D3D11 NVENC and CUDA NVDEC diagnostic paths using the NVIDIA SDK 13.1 ABI. The default desktop codecs and v8 wire protocol are unchanged. Use `desktop-video-adapters` to identify the local NVIDIA DXGI adapter, then pass its explicit index; the encoder/decoder require a matching NVIDIA adapter and a compatible driver. There is no software fallback or runtime FFmpeg requirement.

```sh
cargo build --locked --release -p intralink --features nvenc-preview,resource-monitor
intralink desktop-nvenc-check --height 1080 --samples 180 --adapter-index NVIDIA_INDEX --decode
```

The diagnostic generates synthetic frames only. It verifies H.264 encoding, clean decode callbacks, frame order and sampled NV12 colors. It is not an end-to-end desktop streaming benchmark. Optional independent verification: `python tools/native_codec_verify.py --executable PATH_TO_EXE --adapter-index NVIDIA_INDEX` (requires installed FFmpeg and ffprobe). See [HARDWARE.md](docs/HARDWARE.md) and [APP-VALIDATION.md](docs/APP-VALIDATION.md) for measured results, driver-status caveat, and remaining production gates.

### Isolated H.264 QUIC diagnostic

The optional `nvenc-preview` build also provides `desktop-h264-check --height 1080 --samples 180 --adapter-index NVIDIA_INDEX --precise-timer`. It generates native H.264 packets, sends them over authenticated direct QUIC loopback using the separate `intralink/view/9-h264` profile, verifies byte identity and decodes the matching fixture with native NVDEC. The optional private high-resolution Windows timer avoids coarse default waits. This is a synthetic local transport check, not live desktop sharing, untrusted H.264 decoder certification or an actual renderer. The normal desktop CLI remains v8 JPEG/Tiles/Compact only. See [PROTOCOL.md](docs/PROTOCOL.md), [PERFORMANCE.md](docs/PERFORMANCE.md) and [APP-VALIDATION.md](docs/APP-VALIDATION.md).

### Selected NVIDIA capture and local GPU presentation

Build with nvenc-preview and use desktop-video-adapters to identify the current NVIDIA DXGI index. The generated-window-only command `desktop-wgc-codec-check --height 1080 --samples 180 --adapter-index NVIDIA_INDEX --present` explicitly creates the WGC pool on that adapter, runs GPU NV12/NVENC/NVDEC and optionally presents via CUDA-D3D11 plane copies and a visible native DXGI window. The test reads only sample pixels to validate color. Run `python tools/native_wgc_codec_verify.py --executable PATH_TO_EXE --adapter-index NVIDIA_INDEX --present --repeat-1080 --output NEW_REPORT.json` to require actual process exit codes. See HARDWARE.md and APP-VALIDATION.md for driver teardown evidence and limitations. Normal sharing remains v8, not an enabled physical H.264 session.

### Continuous native GPU/QUIC diagnostic

Build with nvenc-preview. On Windows with an explicitly selected NVIDIA adapter, run `desktop-wgc-live-check --height 1080 --samples 180 --adapter-index NVIDIA_INDEX` to generate a disposable moving test window, capture/encode on one native thread, send over authenticated loopback QUIC and decode/present on a second native thread. Every received packet must match its independent native origin token; no arbitrary network AVC or physical monitor capture is enabled. Use `--cancel-after 30` to test local cancellation, or `python tools/native_wgc_live_verify.py --executable PATH_TO_EXE --adapter-index NVIDIA_INDEX --output NEW_REPORT.json` for a process-level gate. See docs/PROTOCOL.md, SECURITY.md and APP-VALIDATION.md. Normal desktop sharing remains v8-only.

The continuous WGC diagnostic also supports `--isolated-decoder`, which launches the same executable as a supervised private native decoder child. The parent checks the native-origin token before IPC; the child validates the profile and every v9 envelope independently. Use `tools/native_wgc_live_verify.py --isolated-decoder` with the existing executable, adapter and output arguments to verify distinct matching child PIDs, native counts and actual exit codes. Process containment is not a security sandbox or physical remote H.264 support.

### Decoder process hardening

The `--isolated-decoder` diagnostic now applies/readbacks process-local image-load, extension-point and child-creation restrictions, plus a 2 GiB committed-memory job limit. The token-only `desktop-decoder-token-check` verifies a reduced primary token without initializing the GPU or changing the parent. The explicit `--restricted-decoder` (requires `--isolated-decoder`) is **experimental and currently not qualified on the tested hybrid NVIDIA laptop**: the Windows process starts but its DLL initialization fails with 0xC0000142 before Rust entry. It fails closed, not by silently selecting the standard process. This is not a complete sandbox, physical sharing, or arbitrary remote AVC support. See docs/SECURITY.md and APP-VALIDATION.md.
