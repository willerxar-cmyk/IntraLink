# Negotiated software media v11 and native output retargeting

Status: opt-in development preview. The v8 fixed and v10 explicitly matched adaptive software protocols remain byte-compatible. v9 is still a loopback-only isolated H.264 diagnostic. No native hardware codec is advertised on a remote session.

## Local authorization and software capability selection

Both endpoints select `--negotiate-codec` instead of `--adaptive-resolution`; v11 also includes adaptive resolution. The `--codec` setting is an explicit local ceiling in v11: jpeg enables JPEG only, tiles enables Tiles and JPEG, compact enables Compact, Tiles and JPEG. All three are implemented by the software encoder/decoder and are independent of CUDA. This is a local allow-list, not a claim that a device has hardware decoding or a particular measured performance. Both sides independently choose the richest mutually enabled mode: Compact, then Tiles, then JPEG. No common mode, unknown mask bits, invalid selection, missing or mismatched confirmation, wrong peer or version mismatch fails closed. There is no automatic fallback between protocol versions.

The Host announces the permitted set before approval. The existing pinned peer, local consent, selected display, red stop indicator and bounded read-only session remain mandatory. Encoder allocation and capture initialization occur only after the authenticated selection is confirmed; the transport hands the immutable selected profile to the local capture owner. The Host verifies that it differs from local approval only by an allowed software codec. The chosen mode remains fixed until disconnect. The Host's approved resolution ceiling (1080p default), FPS ceiling, quality ceiling and duration are preserved; the existing v10 adaptation policy adjusts height and JPEG quality. Viewer-side resolution/FPS capability limits and vendor-specific hardware codec negotiation are not implemented by this exchange.

Example with already paired devices and local approval:

```
intralink share-desktop --bind <host-address:port> --controller <paired-name> --monitor <local-id> --negotiate-codec --codec compact
intralink view-desktop --device <paired-host-name> --negotiate-codec --codec jpeg
intralink workspace --device <paired-host-1> --device <paired-host-2> --negotiate-codec --fullscreen
```

The first example offers all three software modes and the second accepts JPEG only; both select JPEG. The workspace flag applies independently to every selected host, all of which must use the same protocol version. Hardware capture and scaling are separate local choices (`snapshot`, `wgc`, `wgc-gpu`); explicitly selected hardware capture still fails closed on unsupported operations, rather than silently weakening the requested backend or isolation.

## Wire exchange

ALPN `intralink/view/11-negotiated`. After authenticated peer validation:

1. Viewer sends exactly 17 bytes: ASCII `INTRALINK/11VIEW` (16 bytes), plus a nonzero one-byte capability mask. Bits 0/1/2 mean JPEG/Tiles/Compact respectively; bits 3-7 are reserved and rejected. A valid subset may contain any supported combination.
2. Host intersects the offer with its locally allowed mask and selects the highest ranked common mode. It sends nine bytes: local allowed mask and the existing eight-byte VideoConfig with selected mode. All configuration bounds apply unchanged.
3. Viewer validates the mask, configuration and deterministic intersection, then echoes the exact nine bytes. Host compares the complete confirmation before issuing any capture request. The bounded handshake is separate from record receipts. This does not prove the peer's actual performance or honesty.
4. Changed media records, height, complete-keyframe transitions, repeat/no-update/end, deadlines, receipt credit and order follow v10 unchanged. No mid-session codec negotiation, new remote commands, privileges or arbitrary native H.264 input.

## WGC dynamic output scaler

Only an explicitly enabled GPU-scaled capture session may retarget its local output. Allowed canvases are 1920x1080, 1280x720 and 960x540, not exceeding the approved starting ceiling. The source monitor/window, native geometry and device remain fixed. Snapshot and WGC without GPU scaling continue using their existing software resizing paths. The local adaptation policy chooses the target, never a remote handle or untrusted texture descriptor.

A new target is applied to the next newly observed generation under the existing callback lock. Compute the aspect-preserving fitted size; construct the replacement native processor on the same validated capture device, then release the obsolete scaler/staging resources. The staging reset retains the owning device identity and allocation counters. Other unexpected readback geometry or device changes remain errors. CPU RGBA allocation is recycled, and the encoder resets the tile base to require a complete keyframe. No stale generation, GPU fallback or capture item recreation is introduced. Resource creation failure ends the session. The policy's transition limits bound resource churn.

The Windows-only `desktop-retarget-check` diagnostic creates only its own generated child window, tests five output tiers (1080/720/540/720/1080), six native samples per tier, native dimensions and colors, decoder dimensions, complete keyframes, staging/processor counters, invalid target and stop behavior. It does not capture physical displays or exercise an actual remote session. See the benchmark and handoff for observed results.
