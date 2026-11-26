# Multi-device workspace — v0.17

Development preview: the dependency-free Rust protocol, pacing and workspace policy passed 13 tests on Windows in v0.15; six new presentation tests passed in v0.16. In v0.17 six overview/renderer tests and six presentation regressions passed in a standalone Windows harness. The native viewer and QUIC integration are not compiled or tested end-to-end. Only authorized test networks and explicit foreground sessions; no keyboard/mouse forwarding, unattended host, mobile controller, or remote monitor switch yet.

## User workflow

Pair each device using the existing encrypted vault and independently verified identities. On each host, select its monitor with `displays`; locally approve one `share-desktop` session (`--fps 60 --height 1080 --seconds 3600` is the upper profile, not a performance promise). Start the controller within the 120-second acceptance window.

```sh
cargo run -p intralink --features desktop-preview -- --vault main.vault workspace --device workstation --device notebook --device server --fullscreen
```

The saved names are examples; use actual outbound grants. Up to eight unique host identities. The CLI starts the selected set concurrently. F1–F8 select a slot without disconnecting others. F9 stops only the selected connection and decoder; the shared endpoint remains alive; after cleanup, F9 starts a fresh attempt (the Host must approve a new session). F10 selects adaptive or all-live mode; `--all-live` starts in the latter mode. F11 toggles a borderless monitor-sized window (primary local monitor or `--local-monitor` ID with `--fullscreen`). F12 while fullscreen cycles available local monitor IDs, without changing the captured host display. If a window transition fails, keep the existing window and sessions. Tab opens/closes a local overview; click a card to focus it or its ON/OFF control to toggle that slot independently. The panel includes all-live/adaptive, fullscreen and back controls. Escape closes the overview first, then exits the workspace; closing the window stops all. Monitor geometry/DPI, focus and window-manager behavior need native tests; this is not exclusive fullscreen. Touch/mobile support is not implemented. Per-session journal is not wired for the workspace command.

## Runtime architecture

- The workspace owns one outbound-only endpoint across all independently authenticated connections. Each active slot resolves its own saved route. Closing a slot does not close the endpoint. No central server or catalog is disclosed to a host.
- Each session has a capacity-one ordered changed-record queue, capacity-two recycled compressed-buffer pool and one blocking Rust decoder worker. Encoded deltas are never dropped. A worker updates its independent canvas in order. The presenter can skip displaying intermediate **decoded** canvases, not applying intermediate deltas.
- One window displays the selected canvas. It tries the surface lock, never waits for a decode; selection changes clear stale presentation. An immutable snapshot is taken under the lock and native presentation happens after releasing it. The decoder reuses its canvas when the snapshot has been dropped; if it is still held, `Arc::make_mut` clones the canvas before applying the ordered update. The single presenter holds at most one transient snapshot; an overlapping update can temporarily require one additional 1080p canvas (about 7.91 MiB). Eight base canvases alone are about 63.3 MiB. This is not end-to-end zero-copy. Decoder failure invalidates the surface and prevents a partially decoded update from being published. A failed window transition preserves the previous window. Recreating or moving the local window does not recreate network connections.
- A host-approved FPS cap of up to 60 applies independently to each stream. Foreground and all-live modes retain static-scene pacing. In adaptive mode, ordered ACK feedback caps background sampling at 5 FPS (or lower host cap). A focus change propagates on subsequent receipts, not instantaneously; the selected canvas can already be shown. Nominal feedback interval is up to 200 ms in background plus processing/network delays; short-lived changes may be missed.
- Eight outstanding media records rather than three bound a conforming peer's unacknowledged payload at 16 MiB per connection, excluding QUIC/OS and other buffers. At high RTT the credit ceiling is still finite; actual FPS is not guaranteed. Backpressure can add latency; benchmark for congestion and stale queued data.
- Each host authorizes up to 3600 seconds (default 300). No automatic renewal or reactivation; the host's stop window remains visible. The vault is locked for the workspace lifetime. There is no background service or automatic retry after a session ends.

## Acceptance gates

1. Full Cargo resolution/lockfile review, formatting, Clippy, workspace and optional desktop build/tests. Run the three-host loopback regression, including rejection of one independent host.
2. Real Windows and Linux/X11 tests with three machines. Verify each Host must consent; disconnect/reconnect one, refuse one, switch foreground repeatedly, switch fullscreen and cycle local monitors with F12, exercise unavailable/disconnected displays and rejected window creation, close the window, resize and test high DPI and monitor disconnect. Check that others remain connected, no wrong-surface exposure and no lingering listener or decode worker after stop.
3. Compare 30 and 60 FPS at equal resolution/quality, moving windows/video/static content, 1/3/8 sessions, adaptive vs all-live, LAN and impaired links. Record actual capture, encoding, decode and presentation FPS independently, frame age p50/p95, CPU/GPU/RSS, throughput, dropped presentation frames and focus-switch latency. Verify visible consent while sharing, including remote disconnect.
4. Implement event-driven capture/native GPU codecs, authorized input forwarding, native multi-monitor negotiation, mobile touch/keyboard and fullscreen surface, secure platform key storage, session management and explicit user-approved longer access. None of these are complete. Do not declare a production remote-control product or publish before security/native validation.

## v0.16 presentation validation

The actual `presentation.rs` module passed six independent Rust tests (ordering, allocation reuse, immutable overlap, geometry, failed decode and pixel count) on Windows. A standalone synthetic three-canvas microbenchmark tested 240 rounds with and without forced overlapping snapshots. It excludes real capture, network, codecs, GPU and native presentation; see [PERFORMANCE.md](PERFORMANCE.md). Full workspace integration, native multi-monitor behavior, remote monitor selection and sustained 60 FPS remain release gates.

## v0.17 local overview

- The overview reuses a single lazily allocated 960x540 `u32` canvas (2,073,600 bytes); thumbnails are nearest-neighbor, letterboxed from the existing ordered decoded surfaces. No independent video request or re-encoding. The window refreshes the overview at a nominal 100 ms interval and immediately on relevant local actions; it does not lower the negotiated session frame ceiling. Each thumbnail uses a transient immutable snapshot; only one is held during its resampling. When a surface is locked, unavailable or invalidated, the card displays a waiting state instead of reading incomplete pixels.
- Activation and focus are distinct local actions. Turning a slot off requests closure of its existing session and clears its preview at the next overview render; turning it on initiates a new authorized connection only after previous cleanup. This does not authorize automatic renewal or unattended access. Inactive/closing slots do not display cached thumbnails, and the overview clears its buffer on close. No thumbnail is written to disk. Bitmap labels use ASCII and substitute '?' for unsupported glyphs; the full selected name remains in the native window title.
- Run native manual tests with 1, 3 and 8 actual authorized sessions: verify Tab/Escape, independent ON/OFF, all-live/adaptive, fullscreen/windowed transitions, clicking card bodies and controls, resizing, DPI changes, portrait and ultrawide local windows, simultaneous keyboard/pointer events, closure during decoding and no wrong-device or stale preview after stopping. Check input alignment against the actual native backend; pure geometry tests do not verify DPI, scaling or native input. Benchmark the overview's actual render cost and CPU/RAM usage with motion, locked surfaces and high resolution. Native desktop and all Cargo gates are still open.


## v0.18 measured workspace observations

Each slot now shows local RX, DEC and VIEW changed-frame rates and payload Mbps. The foreground window title also shows these observations. See [TELEMETRY.md](TELEMETRY.md) for exact semantics, limitations and native acceptance gates. A busy decoder retains the last complete thumbnail instead of flashing a waiting card. Slot reuse, invalidation and disconnect clear it. No added full-resolution cache, network stream or protocol change.

## Local native UI smoke test

Run `intralink desktop-check` from the combined-feature Windows or Linux X11 build. It submits a local inactive-device overview through the actual native window factory, checks windowed and borderless fullscreen geometry, optionally cycles to a second local display, then restores the window. No Host is connected and no screen is captured. This is an executable smoke test, not physical-session, monitor-position, DPI, interaction, or 60 FPS certification.

## v8 no-update observations

The v8 desktop wire protocol distinguishes no native update observed from verified unchanged pixels. All records consume ordered credit, but only changed records enter the decoder/presentation queues. One second of continuous no-update observations triggers a local last-frame notice in the selected window/fullscreen and a separate overview footer for that slot; another accepted media frame or verified repeat clears it. The banner copies into a single reused local presentation scratch buffer only when the displayed sequence/notice state changes (up to one additional 1080p u32 canvas); the decoder surface is untouched. A banner-only repaint is not a new decoded or presented sequence. No-update does not certify health and does not change the active-device policy. Both peers must support v8; native multi-device network performance still needs physical validation.


### Optional adaptive-resolution sessions
The workspace accepts `--adaptive-resolution` for all selected hosts. Each must separately opt in on its locally approved Host. The v10 software decoder accepts geometry changes only on validated complete keyframes. Every slot keeps its independent bounded queue, decode state, connection, cancellation and immutable presentation snapshot. A changed geometry uses a fresh or uniquely owned canvas; an older snapshot remains immutable. Background receipt priority remains unchanged; neutral observations do not drive resolution changes. Mixed v8/v10 host selection and physical eight-device qualification remain pending. See [ADAPTIVE-MEDIA.md](ADAPTIVE-MEDIA.md).


### Negotiated codec per session (v11)

`--negotiate-codec` opts every selected slot into v11. Each independently approved Host negotiates its own software codec with the controller's local `--codec` ceiling. The workspace still owns independent decode state, queues and cancellation per slot; no global GPU requirement or change to the authorization scope. Mixed protocol versions in one workspace are not yet supported.


### Per-device resource ceilings (v12)

Choose `--negotiate-profile` and optional global `--max-height` / `--max-fps`. Repeat `--device-limit NAME=HEIGHT:FPS` for selected paired-device names to lower individual slots (e.g. host-2=720:30 and host-3=540:15). The defaults remain 1080p/60 FPS. Reject unknown/duplicate/malformed overrides or widening of the global ceiling; preserve each slot's independent settings across local stop/reactivation. Existing protocols remain selectable; mixed versions remain unsupported. See [BOUNDED-MEDIA.md](BOUNDED-MEDIA.md).
