# IntraLink

**One workspace. Your devices. Direct, authenticated connections.**

IntraLink is an independent Rust remote-access project designed for a single controller, several computers, and optional host/controller roles on the same device. It uses Iroh QUIC with pinned device identities, without a mandatory account, directory or commercial relay.

[Download desktop preview](https://github.com/willerxar-cmyk/IntraLink/releases/tag/v0.22.0-preview.4) · [Quick start](#quick-start) · [Security](docs/SECURITY.md) · [Roadmap](docs/ROADMAP.md)

> **Development preview 0.22.0-preview.4 (Windows and Linux x64).** Windows native generated-window diagnostics and Linux CI builds/tests have passed; Linux physical capture and KVM are not qualified. This is not a production-certified remote-control tool. Use only on private, owner-authorized test networks. Physical multi-device sessions, sustained 1080p60, code signing, and independent security review are still open.

## What works today

| Capability | Status |
|---|---|
| Direct encrypted QUIC, pinned identities, encrypted local vault | Implemented; automated and loopback tests |
| Manual pairing, signed invitations, local grant revocation | Implemented |
| Windows screen-view host and viewer | Preview; generated-window native and loopback validation |
| Software Compact/Tiles/JPEG, 540p–1080p, up to 60 FPS target | Implemented; no NVIDIA codec required |
| Workspace with up to eight independent device slots, fullscreen, overview | Preview; three-host loopback validated, physical device matrix open |
| Adaptive resolution/quality and explicit bounded profile negotiation | Implemented in supported matching wire profiles |
| Input-only mouse, keyboard, wheel, up to eight hosts | Experimental Windows preview; focused pad, explicit F-key or spatial zone switching, physical acceptance open |
| Seamless physical desktop edge switching and simultaneous Hybrid control | In development; not a global hook or unattended control |
| macOS, Wayland, Android, iOS host/controller adapters | Not distributed in this release |

The Windows portable build uses optional WGC/D3D11 capture and scaling; the software codec does not require CUDA, NVENC or a dedicated NVIDIA card. The scalar CPU codec path remains separately tested. No background service, unattended control, arbitrary shell or automatic firewall changes.

## Choose your device role

- **BASE - Control other devices:** use the central computer with your keyboard and mouse. The current CLI calls this role `controller`.
- **CLIENT - Receive control:** use the device that shares its screen or receives approved input. The current CLI calls this role `host`.

Preview.4 adds a graphical first-run BASE/CLIENT selector, named encrypted identities, and authenticated role changes (see [ONBOARDING.md](docs/ONBOARDING.md)). The desktop selector hands off name/passphrase entry to the local terminal; this is not yet a complete GUI dashboard. Screen and input permissions remain independent.

| Platform | Available distribution |
|---|---|
| Windows x64 | Preview 4 portable ZIP; local native diagnostics passed, physical KVM pending |
| Linux x86_64 | Preview 4 TAR.GZ; native Ubuntu 24.04 CI build/tests passed; optional X11 input host and BASE source under qualification |
| Android | Preview.15 candidate; signed pairing and local QR image import, dedicated emulator acceptance; physical Preview.15 and signing pending |

Android source and candidate build/validation instructions: [ANDROID.md](docs/ANDROID.md). Preview.6 CI passed local identity, manual pairing, foreground authenticated diagnostics and local consented AVC encoder build/package gates. The shared Rust [AVC packetizer](docs/AVC-PACKETIZER.md) passed host and authenticated diagnostic loopback tests. Preview.7 adds direct-buffer Rust NAL observation, with actual local encoder output tested on a dedicated emulator. Preview.8 adds output SPS/CSD and PTS observations; the [bounded AVC producer and local exact-mode policy](docs/AVC-PRODUCER.md) passed diagnostic tests, and preview.9 tested local sync-frame observation and a native software decoder. [Physical Windows/Android LAN acceptance](docs/ANDROID-LAN-ACCEPTANCE.md) validates generated Compact frames and diagnostic probes on the actual network; [Preview.10 experimental read-only screen sharing](docs/ANDROID-SCREEN-PREVIEW.md) is implemented with independent foreground consent; owner-approved physical MediaProjection and graphical Windows preview passed; extended performance, lifecycle and distribution qualification remain pending. [Portable motion preview.12](docs/PORTABLE-MOTION.md) adds payload-aware JPEG policy, exact Compact recovery and reusable desktop JPEG output, with generated physical comparisons and native AVC admission diagnostics. [Preview.13 connection cards and KVM experience](docs/KVM-EXPERIENCE.md) introduce simplified public-field exchange, a foreground Android app-only input canvas with a visible cursor, and rearrangeable Windows spatial input zones with a short edge portal. Current source adds named device cards, a saved Windows spatial arrangement and portal preference, and an Android card-first dashboard. System-wide Android input, native desktop edge crossing and simultaneous screen/input remain open.

## Physical desktop edge handoff preview (current source)

On Windows use `input workspace --choose --geometry --detect-displays --desktop-edges` with independently approved input/2 CLIENTs. Review the layout, explicitly arm the local indicator, and approach a shared physical edge. An owned temporary foreground surface captures local input only while remote; if Windows refuses focus, confirm locally or the handoff is canceled. Return through the shared edge after release ACK. F12/Escape stops in the focused control window. This is an experimental foreground source, not transparent global hooks or a physically qualified production release.

## Physical edge calibration and arrival cue (current source)

On Windows, use `input edge-check --device NAME` after saving the exact selected topology. Explicitly start the finite read-only check to validate real desktop edges without opening a connection. On an authorized Windows input/2 CLIENT, optionally add `--entry-portal full` or `--entry-portal reduced` to the input host command to show a small nonactivating arrival animation. These are preparation for global physical switching, not a global input hook.

## Authenticated display detection (current source)

On a Windows CLIENT, explicitly enable `input host --display-geometry` with the existing bind/controller arguments and local approval. On the Windows BASE, `input workspace --choose --geometry --detect-displays` receives the selected pinned hosts' current primary display metadata and opens the proportional editor. Review positions and save; the workspace starts SAFE. No fallback to input/1 or new input grants. The legacy manual geometry mode remains available.

## Display topology (current source)

On Windows, `input topology --device NAME` opens the offline proportional display editor. It reads the primary BASE metadata, asks for actual CLIENT dimensions/scale, and saves a reviewed identity-bound layout. `input workspace --choose --geometry` reviews this arrangement before opening the focused, input-only workspace. Pointer handoff uses real shared logical edges with the existing independent session approval and ACK ordering. This remains a window-scoped preview, not a physical desktop hook. See [input sharing](docs/INPUT-SHARING.md).

## Choose connected devices (current source)

For read-only screen sharing, Windows/Linux desktop-preview now supports `workspace --choose`. Choose up to eight previously paired CLIENTs; the screen picker uses saved routes, not input availability announcements. Each CLIENT still approves its own screen session.

On each paired CLIENT, explicitly approve an input listener with `input host --bind HOST_PRIVATE_IPV4:45100 --controller base --advertise-lan`. On a Windows BASE, use `input workspace --choose` to open the native selector, choose up to eight paired CLIENTs, continue through local approval and review the saved spatial layout. Announcements are untrusted route hints, not authenticated readiness; the existing saved route remains usable. The BASE must explicitly arm its pad. Linux X11 supports `input workspace --device NAME --lan` with the same discovery boundary. See [input sharing](docs/INPUT-SHARING.md). These features are in current source, not the public Preview.4.

## Graphical pairing (current source)

Use `connection-card --gui` to show an unsigned public connection card. For signed bilateral enrollment, use `invite --gui`, `accept-invite --gui` and `confirm-invite --gui` with the existing required parameters. Review both full identities and approve each operation locally. The QR carries the full signed code, not permission to start a screen or input session. Passwords and pasted code entry remain in the terminal. See [secure enrollment](docs/ENROLLMENT.md). These newer source features are not included in public Preview.4.

## Install on Linux (experimental)

Download the Linux x86_64 TAR.GZ and verify the published SHA-256 manifest. Extract into a private user-owned directory. This candidate was built and tested on Ubuntu 24.04; system libraries are not bundled. Check the included linux-runtime-libraries.txt for the native dependency list. On Ubuntu, the runtime libraries include libgbm1 and libxcb1. An active supported X11 session is required for the desktop viewer; Wayland is not qualified. Do not enable global input permissions or run with elevated privileges.

From the extracted directory:

    ./intralink --version
    ./intralink --help
    ./intralink setup

The public Linux preview.4 does not include input KVM. Later opt-in X11 receiving host and focused BASE adapters passed isolated Xvfb and native Linux candidate CI (see [INPUT-SHARING.md](docs/INPUT-SHARING.md)); they are not included in the public preview.4. Physical KVM, Wayland and physical desktop-edge input remain pending. The native CI gate tests the actual binary and transport; it is not a physical desktop acceptance test. Read the license provenance notes included in the package.

## Install

1. Open [Releases](https://github.com/willerxar-cmyk/IntraLink/releases) and select the current Windows or Linux x64 **pre-release**. The Windows instructions below use PowerShell.
2. Download the portable ZIP, verify its SHA-256 against the release manifest, and extract it to a user-owned directory. No installer or administrator rights required. The executable is not yet code-signed; verify provenance before execution.
3. Open PowerShell in that directory and run:

```powershell
.\intralink.exe --version
.\intralink.exe --help
.\intralink.exe desktop-profile-check --pipeline
```

The last command uses an owned, labeled local test window, not your real monitor. The executable and license notices are included together. Do not bypass operating-system security warnings or install third-party driver packages merely to run this preview.

## Quick start

Use two devices on the same trusted private LAN. In the commands below, replace `HOST_IP`, `HOST_PUBLIC_ID`, `CONTROLLER_PUBLIC_ID` and `DISPLAY_ID` with actual values. Use a strong passphrase when prompted; no passwords or private keys belong in command arguments or on GitHub.

### 1. Create identities and pair

On the host:

```powershell
.\intralink.exe init --role host
.\intralink.exe identity
```

On the central/controller computer:

```powershell
.\intralink.exe init --role controller
.\intralink.exe identity
```

Exchange and independently verify the **complete public IDs**. On the host:

```powershell
.\intralink.exe pair --name central --direction inbound --peer-id CONTROLLER_PUBLIC_ID
```

On the controller:

```powershell
.\intralink.exe pair --name workstation --direction outbound --peer-id HOST_PUBLIC_ID --host HOST_IP --port 45100
```

Type `APPROVE` locally when asked. A saved pairing identifies an allowed peer; it does not authorize unattended screen or input access. The encrypted vault is stored in the OS application data directory by default; use `--vault PATH` before the command for a separate profile. An existing profile can use `identity` and `devices` without recreating it.

### 2. Test the direct connection

Host:

```powershell
.\intralink.exe listen --bind HOST_IP:45100 --controller central
```

Controller:

```powershell
.\intralink.exe connect --device workstation
```

Approve on the host and connect promptly. A successful probe verifies the pinned peer identity; it grants no desktop or input authority. This one-shot listener closes after the probe.

### 3. View an approved screen

Host: choose the actual display shown by `displays`. Use a disposable display without sensitive content.

```powershell
.\intralink.exe displays
.\intralink.exe share-desktop --bind HOST_IP:45100 --controller central --monitor DISPLAY_ID --capture wgc --height 720 --fps 30 --seconds 300
```

Controller:

```powershell
.\intralink.exe view-desktop --device workstation
```

Approve each host session locally. Stop using Escape or the visible sharing window; Ctrl+C terminates the foreground process. If WGC is unavailable, start a new explicitly approved session using `--capture snapshot`; there is no silent fallback. Start at 720p30 for the initial physical test and raise to 1080p60 only if the devices can sustain it. Matching codec/protocol options are required on both sides.

### 4. Connect multiple devices

Repeat pairing and local sharing approval on each additional host; use each host's actual LAN route. Then start:

```powershell
.\intralink.exe workspace --device workstation --device notebook --device server --fullscreen
```

Tab opens the overview. F1–F8 switch slots, F9 toggles the selected slot, F10 changes adaptive/all-live mode, F11 toggles fullscreen, F12 selects the next local monitor. Escape closes the overview or workspace. Every slot has independent media state and cancellation. A new host session requires fresh approval. The current video viewer is read-only. Input is a separate capability and does not become enabled by viewing a screen.

### 5. Try input-only software KVM (experimental Windows build)

Use the separately labeled build with `input-preview`. On each distinct paired host, close any existing screen listener using the same vault and port, then run:

```powershell
.\intralink.exe input host --bind HOST_IP:45100 --controller central --seconds 300
```

Approve keyboard/mouse access explicitly on the host. A red stop window remains visible. On the central computer:

```powershell
.\intralink.exe input workspace --device workstation --device notebook --spatial
```

In current source, arrange the named devices while SAFE. The last layout and portal preference are saved for the same authenticated BASE and exact CLIENT set, independent of selection order. Review the layout and click START or press F9 from the local BASE card. RESET restores defaults. Move the pointer between regions with keys/buttons released. F12 disarms and returns local; Escape closes. Omit `--spatial` for explicit F1–F8 selection. No screen transmission is required. This is a focused in-window KVM, not global desktop-edge switching. **Physical keyboard/mouse acceptance on two real devices is still pending**; use only on distinct, nonsensitive, owner-approved private-LAN desktops. See [INPUT-SHARING.md](docs/INPUT-SHARING.md) and [input queue and cancellation](docs/INPUT-QUEUE.md).

See [desktop options](docs/DESKTOP.md), [workspace](docs/WORKSPACE.md), [signed invitations](docs/ENROLLMENT.md), [networking](docs/NETWORKING.md), and [protocol compatibility](docs/PROTOCOL.md). A reachable direct UDP route is required. DNS does not bypass NAT/CGNAT or restrictive firewalls; internet exposure is not qualified.

## Build from source

Use stable Rust, Cargo, rustfmt, Clippy and the native prerequisites documented in [BUILD-VALIDATION.md](docs/BUILD-VALIDATION.md). The checked-in Cargo.lock is authoritative.

```powershell
cargo fmt --all -- --check
cargo test --locked --workspace --features intralink/desktop-preview,intralink/jpeg-parallel,intralink/resource-monitor
cargo clippy --locked --workspace --all-targets --features intralink/wgc-preview,intralink/jpeg-parallel,intralink/resource-monitor -- -D warnings
cargo build --locked --release -p intralink --no-default-features --features wgc-preview,jpeg-simd,jpeg-parallel,resource-monitor
```

For a Windows/X11 software desktop build, use `desktop-preview` rather than `wgc-preview`. On Windows, add `input-preview` only when explicitly testing the foreground input mode. Omit `jpeg-simd` to test scalar encoding. Optional NVIDIA and VA-API diagnostics are not the standard portable release.

## Architecture and development

- `intralink-core`: portable protocol, media, pacing, workspace and policy.
- `intralink-transport`: authenticated Iroh QUIC and bounded session state.
- `intralink-vault`: encrypted local profiles and directional grants.
- `intralink-d3d11`: isolated optional Windows graphics boundary.
- `intralink-input-win`: isolated, optional Windows input application boundary.
- `apps/intralink`: foreground CLI and native preview windows.

See [latest validation](docs/APP-VALIDATION.md), [performance](docs/PERFORMANCE.md), [architecture](docs/ARCHITECTURE.md), [changelog](CHANGELOG.md), [security](docs/SECURITY.md), and [contributor handoff](docs/NEXT-MODEL-PROMPT.md). Raw measurements are preserved in `benchmarks/`. The fixed 1080p60 setting is a ceiling, not a promise; capture, delivered frames and presentation are different measurements.

## License

Project source: [MPL-2.0](LICENSE). IntraLink is an independent implementation. Third-party components retain their respective licenses and original notices; review the release dependency/license inventory before distribution.

Please report security concerns responsibly without publishing credentials, peer IDs, enrollment codes, or private screen content. Do not use IntraLink on devices you do not own or administer with explicit authorization.
