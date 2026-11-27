# Desktop preview (v0.17)

Status: native preview source, not built, executed or audited end-to-end. Standalone core tests are recorded in [VALIDATION.md](VALIDATION.md). Windows and Linux X11 remain the initial targets. One binary, optional `desktop-preview` feature; default headless commands do not require it. This is read-only screen sharing, not full remote control.

## Native prerequisites

Install current stable Rust and native development tools. Windows: MSVC build tools and Windows SDK. Linux (Debian/Ubuntu):

```sh
sudo apt-get install pkg-config libclang-dev libxcb1-dev libxrandr-dev libdbus-1-dev libpipewire-0.3-dev libwayland-dev libegl-dev libx11-dev libxcursor-dev libxkbcommon-dev libxkbcommon-x11-dev
```

These are upstream prerequisites, not a validated distro recipe. Build/test the default workspace and optional desktop feature. Native tests require a real interactive OS session. No headless or Wayland fallback. macOS and mobile adapters remain deferred.

## Local use

Follow the README pairing steps with independently verified identities. `displays` lists local monitors. `share-desktop` starts one foreground session for one named paired controller and matching `--codec compact|tiles|jpeg` only after typing APPROVE. The local waiting window appears before listening; it turns red before any capture, which is requested only after pinned peer and protocol validation. Stop through the local window, focused Escape, or Ctrl+C. The viewer uses the saved route and can close independently. A new session needs new local consent. Do not expose the preview publicly. No recording, input injection, file access or background listener. The recipient can save the visible content independently. Do not share sensitive content.

## Limits and behavior

| Property | v0.16 |
|---|---|
| Output | 960x540, 1280x720, 1920x1080; aspect-preserving black bars |
| JPEG alternative quality | Host 35–90, default 75; CPU encoder |
| Rate | Host 1–60 FPS upper target, default 60; static pacing; actual throughput unmeasured |
| Session | Host 1–3600s, default 300; at most duration × target FPS records |
| Record | 2 MiB maximum; unchanged record is four bytes plus four-byte ACK |
| Source | Up to 33,177,600 pixels; native backend may allocate before post-capture check |
| Decode | JPEG, negotiated hybrid tiles or independent lossless Compact keyframe; validated configuration and dimensions, best-effort JPEG allocation limit |
| Backpressure | One capture demand and up to eight outstanding records for conforming peers; one queued changed record; ordered decode without encoded-frame drops |
| Route | Direct saved DNS/IP, no relay, mDNS or auto-reconnect in viewer |

Network I/O runs on a Tokio task, not on the native window/capture loop. Escape/close signals cancellation even while the network waits; a synchronous native capture, resize, encode or decode can still delay event processing. Ctrl+C is the foreground emergency stop. Compare output-size RGB exactly before compression; unchanged repeat means no decoder work. From v0.13, consecutive unchanged records reduce the requested sampling rate, without changing the wire format; native capture and resampling still occur at each requested sample; JPEG and tile updates are CPU-only. Compact mode attempts an exact base first within 128 KiB; JPEG remains a negotiated alternative for complex scenes. The viewer recycles consumed compressed allocations using a bounded pool and repaints to process window events even without a changed image. The fixed eight-record receipt window reduces stop-and-wait while limiting conforming-peer unacknowledged media payload to 16 MiB. It is not a RAM cap, bandwidth guarantee, or proof of rendering. Receipt pump consumes ordered ACKs independently; invalid or missing receipts may still fail after bounded outstanding data or at final drain. QUIC and OS buffers remain. See [PROTOCOL.md](PROTOCOL.md) and [PERFORMANCE.md](PERFORMANCE.md).

The session holds the vault lock; stop the session before revoking. No live revocation or persistent desktop grants. A geometry change stops sharing. Robust OS monitor identity, display topology, lock/sleep policy, native indicators and privilege review remain required. Decoder limits are not a security sandbox. No claims about memory erasure or disk paging. No automatic fallback to v1 through v6. Upgrade both peers.

The experimental controller workspace maintains independent connections and one ordered decoder worker per active device. See [WORKSPACE.md](WORKSPACE.md). It is still one preselected Host monitor per session.

## Required native validation (not run)

1. Run workspace and optional desktop build/tests on native Windows and Linux X11; verify old vaults and probes are unchanged.
2. Exercise approval/refusal, wrong peer, malformed request, two displays, DPI, selected monitor disconnect, lock/sleep, headless and Wayland refusal. Confirm capture starts only after the indicator and correct peer are verified.
3. Verify local and viewer close/Escape/Ctrl+C while waiting for authentication, waiting for network data or ACK, and during native call errors. Verify listener teardown, no new frames on local stop, no disk frame files.
4. Test invalid configuration/headers, truncated or malformed JPEG/tile records, wrong codec, delta before keyframe, repeat before first frame, bad acknowledgment, deadlines, cross-ALPN, zero or excess records. Fuzz the protocol and decoder separately. Audit dependencies.
5. Benchmark according to [PERFORMANCE.md](PERFORMANCE.md) and inspect packet capture for direct-only traffic. Do not infer measured FPS, native support or safety from code inspection.

See [TILES.md](TILES.md) for codec negotiation, frame dependency rules, tile format and the Rust microbenchmark. The single ordered receiver queue trades frame dropping for bounded backpressure; native decode still runs in the viewer window loop.

## Local device overview (v0.17)

Tab opens a local thumbnail overview of the selected workspace's paired device slots. Click a card to focus it, or ON/OFF to activate or stop the slot. A new connection still requires fresh Host approval. The overview is purely local; no keyboard/mouse events are sent to Hosts. The 60 FPS ceiling and v7 wire protocol are unchanged. See [WORKSPACE.md](WORKSPACE.md).

## Windows maintenance: local UI diagnostic and event waiting

`intralink desktop-check` exercises the same native window factory and overview painter used by the workspace. It opens a window, primary borderless fullscreen, the next local display if available, and restores a normal window. The panel contains only labeled inactive diagnostic cards. The command checks native buffer submission and window dimensions; it does not open a vault, capture a display, establish a connection, inject input, or certify physical presentation latency. Closing a diagnostic window or a failed native call returns an error.

The Host now waits asynchronously for its bounded capture request channel with an 8 ms UI event deadline when no request arrives. A request wakes the wait immediately. The local indicator is checked before any capture and again before handing encoded pixels to transport. Actual native capture/encode remains synchronous and its cancellation latency still needs physical testing. Interactive terminal, identity, and explicit local session approval remain mandatory for screen sharing.

## Native local media diagnostic (Windows)

`intralink desktop-media-check --height 1080 --samples 120 --codec compact` creates a disposable child of the same executable showing an animated generated color pattern. XCap 0.9.10 excludes the current process from its Windows window enumeration, so the diagnostic selects only the exact child PID and title and validates dimensions and pattern markers. The production CPU Encoder and Decoder process the native window capture and MiniFB submits the decoded canvas to a local preview. No monitor capture, remote connections, credentials, image files or input control are involved. The diagnostic exits on cancellation, mismatch or native error; the child is cleaned up and has a hard 45-second lifetime. This subprocess is a diagnostic fixture, not a production runtime dependency.

The diagnostic reports stage p50/p95/p99 and actual local sample/changed-frame throughput. Native buffer submission is not physical screen presentation timing. Its window capture uses XCap's window backend, not the production full-display capture path. It is not a physical network or sustained 60 FPS certification.

## Experimental persistent WGC diagnostic

Build on Windows with `cargo build --locked --release -p intralink --features wgc-preview,resource-monitor`. Run `intralink desktop-media-check --height 540 --samples 60 --codec compact --wgc`. The explicitly selected WGC backend starts one capture session for the owned diagnostic child, keeps the system capture border settings unchanged, reads back only requested frames, and stops/joins the session on completion or error. A missing feature or capture failure returns an error rather than falling back silently. No remote Host screen capture, network listener or image recording is enabled by this experimental adapter. The production CPU resizer is shared by all desktop-preview builds.

## Reusable readback instrumentation

`desktop-media-check --wgc` reports `staging_allocations` and `rgba_allocations` after a successful native run. The first counts retained D3D11 staging texture creations; the second counts occasions on which the recycled RGBA vector had insufficient capacity. These are application-level resource counts, not all driver or process allocations. Native source changes fail closed. The fixture-only diagnostic retains the system capture indicator and closes the capture session before destroying its child window.

The Windows `wgc-preview` diagnostic also accepts `--gpu-scale` together with `--wgc`. It explicitly requires the native D3D11 video processor and reports processor creations and cumulative unpadded RGBA bytes read back. A driver error ends the diagnostic, rather than hiding a CPU fallback. No monitor capture or remote session is started.

## Native idle/final-update diagnostic

Build with `cargo build --locked --release -p intralink --features wgc-preview,resource-monitor`. `intralink desktop-static-check --height 540` and `intralink desktop-static-check --height 1080 --gpu-scale` create a test-only child window with an initial marker, one subsequent marker movement, and no further frame submissions. The consumer reads the initial image, makes no readback requests for five seconds across the final change, and verifies the final marker and unchanged readback counters during idle. The test checks exactly two readbacks and one staging texture and one RGBA allocation. It does not prove physical monitor behavior or remote-stream reliability. `desktop-media-check` now reports native updates, readback frames, native samples and no-new-sample polls separately; the measured capture rate counts actual readbacks rather than polling attempts.

### Static marker correction

The first native static trials failed their marker assertion. The revised fixture uses two nonoverlapping interior locations and requires the inactive location to match its exact background, rather than only rejecting the previous marker color. Scaled positive and stale-state negative checks are included. The five-second no-readback interval and exact two-readback/resource reuse checks remain unchanged. The generated-window fixture is not a full-display session.

### Validated diagnostic lifecycle
The final native generated-window checks at 540p and GPU-scaled 1080p passed retention, idle readback, local cancellation and the real WGC Closed event. Both rejected further capture after stop. The production Host still uses its previous backend; monitor topology changes, GPU loss, session authorization and real peer streaming remain validation gates.

### Native geometry and ownership diagnostic

With the optional WGC feature, run `intralink desktop-geometry-check`. This diagnostic first checks the real D3D11 ownership boundary on synthetic texels, then starts a generated child window and requests a resize over a private process pipe only after the first valid capture. The child checks its own window ownership before the isolated Win32 resize. The WGC callback must report a geometry/device change; no readback after the resize is allowed and callback failure must propagate at session shutdown. No unrelated window, monitor configuration, network connection or GPU driver is modified.

## Explicit persistent WGC Host preview (wire v8)

The Windows build with `wgc-preview` now offers `share-desktop --capture snapshot|wgc|wgc-gpu`. Default snapshot remains the previous XCap backend. WGC uses the native latest-surface mailbox; wgc-gpu explicitly requires the D3D11 RGB scaler before CPU readback. No backend silently falls back. The selected backend is printed before the interactive local APPROVE prompt. The Host creates the capture source only on its first demand after authenticated peer and matching protocol validation and after painting the visible red sharing indicator. The existing system capture-border defaults are preserved. No service, unattended access, input forwarding or new grant is introduced.

Display selection requires a unique locally enumerated XCap ID; the WGC adapter additionally requires a unique enumerated HMONITOR exactly representable by that ID and matching device name and dimensions. The approved display name, dimensions, position and rotation are checked again before/after capture and before transmission; there is a periodic check during demand stalls. A different source or changed geometry ends the session. The callback rejects a first frame inconsistent with approved native dimensions. This is OS display identity checking, not a certified physical display hot-plug or lock/suspend policy.

A missing new WGC generation produces a v8 no-update record only after the first valid media record. It does not call the encoder, does not count as a repeat, and does not prove freshness. The Host checks known closure/device failure and conservatively ends the session after 15 seconds without an observed readback (a truly idle source can therefore require fresh approval). This deadline is not a guarantee of native call preemption. The viewer retains its last decoded image, with a visible local last-frame banner after one second of consecutive no-update observations. The workspace overview marks the affected slot and fullscreen renders the same notice. A verified new media record or repeat clears the notice. No notice changes ordered decoding or media counters.

Use only with an explicitly authorized, isolated real test display and paired controller. Both peers must use the same supported protocol (v8 by default, v10 if explicitly enabled). Actual physical WGC full-display consent, native indicator, monitor identity, cancellation, suspend/lock, driver loss, multi-device operation and 1080p60 network testing are not yet certified.


## Portable default and adaptive quality (2026-11-26)

Build a CPU-codec desktop application with `cargo build --locked --release -p intralink --no-default-features --features desktop-preview,resource-monitor`. No CUDA, NVENC or NVDEC feature is required. Snapshot capture is still the default native backend, Compact is the default codec, and the default output height is now 1080 (60 FPS ceiling, not guaranteed performance). Use `--height 720` or `--height 540` for constrained devices, and `--fps` to choose a lower approved ceiling. Actual native capture/rendering requires the respective OS desktop facilities; Windows and X11 are the current preview targets. Other OS adapters remain open.

JPEG quality defaults to adaptive with `--quality 75` as its ceiling and 35 as its floor. The Host announces the range before local approval; a new target is printed and shown in the local stop window. `--fixed-quality` restores fixed quality. After eight consecutive changed records with six or more outstanding receipts, the next JPEG quality target decreases by 10; after 60 changed records with at most two outstanding receipts, it increases by five, never exceeding the approved ceiling. A neutral, static or background sample interrupts the streaks. The target applies to future JPEG keyframes only; no forced refresh, codec switch or lossless tile quantization. Receipt pressure is not measured bitrate or guaranteed receiver rendering. Frame size limits and failure behavior remain unchanged; if a frame still exceeds the wire limit, stop and select a lower profile. Resolution is fixed within v8. The explicit `--adaptive-resolution` v10 mode permits keyframe-only changes between 1080/720/540 within the approved ceiling, with independent local pressure/encode-time hysteresis. The Host and viewer or workspace must both opt in; no automatic protocol fallback. See [ADAPTIVE-MEDIA.md](ADAPTIVE-MEDIA.md).


## Negotiated codec and adaptive GPU output (v11)

Use `--negotiate-codec` on both endpoints instead of `--adaptive-resolution` to select the best mutual software codec automatically and enable adaptive geometry. In this mode, `--codec` is the richest permitted software mode, not a mandatory exact match. The default Host remains 1080p/60 FPS ceiling. v8/v10 remain available as before. An explicit WGC-GPU session now retargets its D3D11 output resources at local adaptive transitions without reopening the capture item; software capture remains supported. See [NEGOTIATED-MEDIA.md](NEGOTIATED-MEDIA.md).


## Bounded profile selection (v12)

Use `--negotiate-profile` on both endpoints to negotiate viewer maximum height/FPS and software codec ceiling. Viewer `--max-height 720 --max-fps 30` cannot raise a locally approved Host cap. Defaults remain 1080p and 60 FPS. Adaptive recovery cannot exceed the selected ceiling. This opt-in mode preserves v8/v10/v11; no automatic downgrade or native H.264 exposure. See [BOUNDED-MEDIA.md](BOUNDED-MEDIA.md).


## Optional measured receiver profile

Use `--negotiate-profile --auto-profile` on a viewer/workspace to calibrate real CPU decoder work locally and choose bounded resolution/FPS before negotiation. The diagnostic `desktop-profile-check --slots N` reports measured tiers without screen capture or network; optional `--hardware` queries and tests native Windows video conversion in the WGC build. Existing Host/capture policy and manual v12 ceilings remain unchanged. See [RUNTIME-CAPABILITIES.md](RUNTIME-CAPABILITIES.md).


## Optional measured Host profile

`--capture auto` selects a concrete locally measured backend and bounded height/FPS on a disposable generated window before the sharing approval prompt. No real monitor capture or network starts during calibration. Explicit backends and the snapshot default remain unchanged; no fallback after approval. `desktop-profile-check --host` runs the same diagnostic independently. See [HOST-CALIBRATION.md](HOST-CALIBRATION.md).


## Optimized portable JPEG encoder

The software JPEG path uses the pinned Rust encoder with optional runtime AVX2 and preserved baseline 4:4:4 chroma/quality. Explicit scalar builds omit `jpeg-simd`. Output is checked while writing against the existing 2 MiB record bound. No wire or authorization changes. `desktop-profile-check --encoder` runs an encoder-only local comparison; see [JPEG-ENCODER.md](JPEG-ENCODER.md).


## Optional ordered Host pipeline

`share-desktop --pipeline --fixed-quality` enables two locally reserved ordered demands on fixed v8. Incompatible with adaptive geometry/codec negotiation in this first version. The visible stop window, approved display and peer, existing wire credit and ordered deltas remain enforced. `desktop-profile-check --pipeline` compares the one- and two-demand paths on a generated WGC window through authenticated QUIC loopback. See [ORDERED-PIPELINE.md](ORDERED-PIPELINE.md).
