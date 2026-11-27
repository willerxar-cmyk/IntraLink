# Input-only software KVM preview

## Goal and current scope

IntraLink separates screen viewing from input control. A device may act as Host or Controller; Hybrid profiles may have independently paired inbound/outbound identities. The first Windows input preview is explicitly enabled by the `input-preview` build feature. It shares mouse movement, three mouse buttons, horizontal/vertical wheel and a bounded physical keyboard subset without transmitting a screen. Up to eight independently connected paired Windows hosts can be selected from a central foreground pad. It is not an unattended input agent, global desktop edge hook, clipboard/file transfer, or simultaneous bidirectional video/input service. A Hybrid can reverse roles in separately authorized sessions; the same vault is locked while a command is active. Same-process multiplexed hybrid roles remain future work.

The current Windows host acts on its existing interactive desktop through the documented SendInput API; no elevation, privileged driver, secure-desktop bypass or global keyboard interception. The controller captures only while its own MiniFB window is focused. It maps its pointer to the remote primary display. Use on distinct physical machines with the local owner present, a trusted private LAN, a nonsensitive desktop and no elevated applications. Do not run multiple input hosts or same-machine controller/host profiles on one desktop. Physical input, international keyboard layouts, modifier interaction with simultaneous local physical input, automatic key repeat, DPI, multi-monitor coordinates, focus and recovery require explicit physical acceptance. A successful injection API return is not proof of application handling or on-screen action.

## First use

Build with `cargo build --locked --release -p intralink --no-default-features --features input-preview,jpeg-simd,resource-monitor`, or use a separately labeled public preview built with that feature. The input adapter does not require WGC, CUDA or NVENC. Follow README pairing: initialize a host and central controller, independently verify full public identities, save directional inbound/outbound grants. The outbound route must be the actual private IPv4 of the host and its explicit input port. Close any screen listener using that vault/port first. Do not expose the port to the internet.

Host (on each distinct physical machine):

```powershell
.\intralink.exe input host --bind HOST_IP:45100 --controller central --seconds 300
```

The owner must unlock and type APPROVE for this distinct input capability. A red, topmost stop window is shown before accepting the single pinned peer. Close it or focus it and press Escape to revoke. Ctrl+C is an emergency process stop; abrupt process termination, OS/driver failure or a physically held local key can prevent perfect release, so keep a local keyboard/mouse and stop access locally if needed.

Controller:

```powershell
.\intralink.exe input workspace --device workstation --device notebook
```

Approve the visible pad, wait for the named paired hosts to connect, focus the pad and place the pointer inside. F1–F8 select the matching device in command order, F12 returns local, Escape closes. The pad sends no input until explicitly activated. Leaving the pad with the pointer deactivates the selected host; losing foreground focus while active disconnects rather than delivering stale events.

Optional spatial mode:

```powershell
.\intralink.exe input workspace --device workstation --device notebook --spatial
```

The numbered grid displays zone 0 (local/safe) and zones 1–8 (command order). Ready hosts are blue; active green; unavailable gray. Press F9 to arm. A target must remain under the pointer for 80 ms with all local keys/buttons released. Crossing to a new zone releases the preceding host before any new activation; moving a held chord or drag across a zone disarms the pad, requiring explicit F9 again. F12 disarms and releases; zone 0 is local; leaving the pad disarms. Escape closes. This is a spatial *in-window* KVM, not yet movement across the physical edges of the controller's real desktop. No global hooks are installed.

## Wire and ownership

- Separate `intralink/input/1` ALPN, pinned identities, private-LAN CLI policy, exact eight-byte hello/ready and bounded host duration. No authority inherited from view/probe; pair records identify devices but a fresh local input-specific approval remains required.
- Fixed 12-byte canonical event records (big-endian sequence, tag, flag, two 16-bit fields, reserved zero). Tags: Activate, Deactivate, Heartbeat, End, normalized Move, physical HID Key, Button, bounded Wheel. No strings, arbitrary OS key codes, shell, files, clipboard or executable data. Reject reserved bits, gaps, invalid values, repeated transitions, inactive input and post-End traffic.
- One outstanding input record per host; ACK is the exact sequence after local sink acceptance. Each controller worker has a bounded 16-event queue. Consecutive unsent absolute moves with no held key/button or acknowledgment may be consolidated; drags, wheel events, key/button transitions and focus barriers remain ordered. Queue residence plus acceptance is bounded to 500 ms for interactive commands; saturation or expiration fails closed, never discards a key-up while silently continuing. See [INPUT-QUEUE.md](INPUT-QUEUE.md). Bound to 512 records per one-second accounting window, 3600 s maximum, 3 s IO/idle deadline, 120 s acceptance. No changes to media wire versions or eight-record media credit.
- The controller has one active remote focus. A switch has a distinct old-release and new-activate confirmation; an old host's release ACK is required before another host can receive input. Each switch command uses the 500 ms interactive budget and checks foreground focus/stop. Inactive connections may maintain heartbeats; active heartbeats and commands require a recent 1 s foreground UI lease, so an independently running worker cannot indefinitely keep an active session alive after UI failure.
- The native input boundary independently validates the ordered state and constructs explicit Windows INPUT variants. Track successfully injected keys/buttons and release only that owned set on Deactivate, End, local stop or ordinary disconnect/error. Native cleanup is best effort under OS rejection or abrupt process termination; no claim of hard preemption or preservation of simultaneous physical key state.
- A failed inactive slot does not automatically activate another. An active failure or uncertain switch aborts the controller workspace rather than risk two active hosts or stale events. Fresh local approvals are required to reconnect. No key content or remote input payload is logged.

## Qualification gates

Test the actual portable parser/state/focus/layout, every supported native scan mapping without OS injection, local reserved controls, UI lease, render geometry, CLI private-network policy, authenticated 3/8-host QUIC, hostile sequence/inactive input, wrong peer, local acceptance rejection and disconnect release plan. Test locked CPU, WGC, NVIDIA, input and independent scalar builds with strict Clippy. Preserve video visual, capture-lock, staged-readback, retarget and geometry regressions. Automated tests use ephemeral loopback with an explicit test sink and never inject into the developer's physical desktop.

Before production or enabling global desktop edge capture, qualify on two and then eight distinct owner-approved devices: verify actual mouse movement/buttons/wheel/keyboard, physical screen-edge topology, focus loss, drag/chord release, disconnect/idle/abort/denied OS input, latency and CPU, keyboard layouts, multi-display DPI, independent inbound/outbound grants, no capture or injection outside the authorized session, and visible local stop. Do not claim physical qualification from loopback or scan-code construction.

## Next steps

Create a native local input-source boundary with explicit opt-in and a reliable emergency release gesture, actual physical display topology and coordinate/edge hysteresis, suppression of injected feedback, raw input/physical-layout semantics and coalesced pointer motion without dropping key/button transitions. Qualify Windows first, then permissioned Linux X11/Wayland, macOS and supported Android/iOS roles. Integrate independently authorized view/input/resource capabilities into one connection manager and support Hybrid direction changes with separately scoped consent, independent revocation and no implicit reciprocal permission. Clipboard, files and audio need their own grants and bounded protocols. See REFERENCES.md; no external KVM source copied or linked.


## Linux X11 receiving-host development candidate

The opt-in input-x11-preview feature adds a native X11 CLIENT (Host) using the existing input/1 protocol. Build without any video or GPU dependency:

    cargo build --locked --release -p intralink --no-default-features --features input-x11-preview

On a distinct, owner-authorized Linux machine with a local X11 login, initialize and pair the controller identity using the existing encrypted vault. Then run:

    ./intralink input host --bind HOST_IP:45100 --controller central --seconds 300

Type APPROVE locally after comparing the complete peer identity. The application opens a managed red stop window before binding the listener. Click the window or press a key inside it to stop. Closing, unmapping or receiving an obscured visibility event also stops the session. The window manager is asked to keep it above normal windows; if it cannot remain fully visible the session is not allowed to continue. Never minimize it, use a sensitive desktop, or leave the owner absent. Windows BASE uses the same input workspace command and ALPN as before. Linux BASE focused native capture is still pending; this feature intentionally exposes host only.

The native crate uses safe Rust XCB checked requests, XTEST 2.2, XKB 1.0 physical key names and RandR 1.5 monitor queries. It refuses Wayland/XWayland, SSH environments, nonlocal display addresses, multiple screens/monitors, incomplete/ambiguous physical keyboard maps, unsupported pointer mappings and invalid geometry. This is a conservative first adapter; no guessed keycode fallback. Mouse coordinates map to the one active monitor covering the X11 root; three buttons and bounded wheel events use the standard X11 button mapping. Geometry/keyboard/button mapping changes cause fail-closed stop. Physical key positions are mapped at the destination; desktop source capture and international layouts still require real-device tests.

Every record is independently validated against the ordered input state. Checked server requests precede ACK, which does not prove the destination application processed the event. Track the original physical code of potentially accepted downs; release nonmodifiers before modifiers and clean up session-owned buttons, including a partial wheel operation. Reject down when the same key/button is already locally held. The X11 server cannot distinguish all concurrent local and injected input ownership; cleanup is best effort if the server hangs, connection dies, process is killed, or hardware/local input changes concurrently. No privileged driver, input grab, background service, access-control changes, or global input capture. The stop window's visibility and actual session-lock behavior depend on the real window manager; do not treat X11 as a security isolation boundary or use with privileged applications.

Native and transport layers are shared through input_host.rs; Windows still uses its prior foreground MiniFB stop window and SendInput adapter. The new X11 candidate workflow runs strict lint, pure tests, an explicitly isolated authenticated Xvfb test for actual XTEST movement/key/button state, release and stop, then the CPU-only application build and CLI tests. The native test is ignored by ordinary cargo test and must only run on a disposable Xvfb display. No physical desktop injection in automated tests. Linux builds without the feature retain their previous behavior. No release promotion until native CI and physical acceptance are reviewed. See SECURITY.md and PLATFORMS.md.
