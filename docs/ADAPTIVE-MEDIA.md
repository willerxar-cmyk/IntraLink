# Adaptive software media v10

Status: opt-in development preview. The v8 fixed-resolution protocol is unchanged. The v9 H.264 diagnostic remains loopback-only and is not a normal sharing codec.

## Authorization and compatibility

Both peers explicitly select `--adaptive-resolution` (Host, single viewer, or workspace). All existing consent, pinned identities, read-only scope, local stop indicator, cancellation, eight-record credit and frame/time limits remain mandatory. This is a geometry extension, not remote input, unattended access, a codec switch or an automatic protocol fallback. A v8 peer cannot negotiate the adaptive-only endpoint. Both endpoints must still select the same codec (`compact`, `tiles`, or `jpeg`). CPU codec baseline works without CUDA or a vendor-specific GPU encoder.

Example with the already paired and locally authorized devices:

```
intralink share-desktop --bind <host-address:port> --controller <paired-name> --monitor <local-id> --adaptive-resolution --height 1080
intralink view-desktop --device <paired-host-name> --adaptive-resolution
intralink workspace --device <paired-host-1> --device <paired-host-2> --adaptive-resolution --fullscreen
```

The workspace flag applies to all requested hosts. Mixed protocol versions in one workspace remain future work. Separate opt-in v11 software codec negotiation is now implemented; see [NEGOTIATED-MEDIA.md](NEGOTIATED-MEDIA.md). Select `--capture wgc` or `--capture wgc-gpu` explicitly on a supported Windows WGC build; the portable snapshot path remains available. Neither mode silently falls back.

## Wire format

- ALPN `intralink/view/10-adaptive`; 17-byte request = 16 ASCII bytes `INTRALINK/10VIEW` + existing codec mode byte (0 JPEG, 1 Tiles, 2 Compact).
- Initial eight-byte `VideoConfig` is identical in layout to v8. Height is the initial resolution and approved ceiling; width is height x 16/9. Allowed heights: 540, 720, 1080. Quality ceiling, FPS ceiling, duration, codec and credit are fixed.
- Changed record: existing four-byte big-endian codec payload length, followed by a two-byte big-endian height, followed by the existing codec payload of the specified length. Codec payload is bounded by 2 MiB; the two-byte geometry field is separate. Media payload statistics exclude the height field, just as they exclude length and ACK headers. End, repeat and no-update records retain their existing header-only representation.
- The first changed record must be a keyframe at the approved ceiling. A height transition is allowed only on a JPEG keyframe or Compact lossless keyframe (if negotiated), after at least 16 changed records at the previous height. Unknown heights, unnegotiated tags, delta before a base or across a geometry boundary, premature transitions, and upward changes beyond the ceiling are rejected. Sender and receiver independently validate boundaries before writing or handing off a record. Existing software decoder validates actual compressed content and dimensions; structural checks do not certify decoder sandboxing.
- No new control request or ACK interpretation. Repeat/no-update records do not alter the current geometry, do not count toward the transition threshold, and never cause decoder reconfiguration. All record order, credit and session limits still apply.

## Local policy

Start at 1080p by default (or lower explicitly approved ceiling). After 16 consecutive changed records with high receipt backlog, or low backlog and local resize/compare/encode work exceeding the approved frame interval, downshift 1080 -> 720 -> 540. After 120 consecutive changed records with low backlog and local encode work at or below half the interval, recover one tier, never beyond the approved ceiling. Neutral/background observations, repeats and no-update reset streaks. These are bounded engineering thresholds, not network bandwidth estimates or FPS guarantees. JPEG quality hysteresis remains independent (default 75 down to 35); `--fixed-quality` disables only JPEG quality adaptation. The Host announces the policy before consent and reports transitions. Single-view title updates on actual dimensions or freshness changes only.

The software encoder reuses its two bounded RGB canvases, invalidates the previous tile base, and forces the next observed changed image to become a complete keyframe. A lower target can reduce software resize/encoding and transmitted pixels, but does not guarantee reduced native capture cost. In `wgc-gpu`, the capture scaler now retargets its output at locally selected adaptive transitions, preserving the native capture item and device. See [NEGOTIATED-MEDIA.md](NEGOTIATED-MEDIA.md) for the resource lifecycle and qualification scope. The single viewer and workspace decoder enforce keyframe-only reconfiguration; immutable presentation snapshots preserve previous geometry and pixels, and malformed partial decode is never published.

## Qualification

Tests cover portable hysteresis, fixed-v8 separation, malformed geometry/mode and transition limits, encoder/keyframe reset, snapshot isolation, real authenticated loopback resolution sequence (1080/720/540/720/1080, 65 complete Compact images), unapproved identity, invalid local delta, raw authenticated peer invalid geometry before consumer/receipt, and v8-to-v10 ALPN mismatch. See the current benchmark log and handoff for actual executed gates.

Still required: real locally consented two-device and multi-device network sessions, forced network/CPU load and automatic recovery measurements, native capture/scale lifecycle under device loss and display changes, platform-specific backends and mutually supported codec negotiation. Keep H.264 native input containment separate. Do not present fixture/loopback throughput as sustained physical 1080p60.
