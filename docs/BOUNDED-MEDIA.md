# Bounded profile negotiation v12

Status: opt-in development preview. Select `--negotiate-profile` on both endpoints. Existing v8 fixed, v10 adaptive, v11 software negotiation, and isolated v9 H.264 byte formats are unchanged. There is no automatic protocol fallback.

## Local ceilings

The Host locally approves its maximum resolution (1080p by default), FPS (60 by default), software codec ceiling, JPEG quality and session duration. The viewer specifies `--max-height 540|720|1080` and `--max-fps 1..60` (defaults 1080 and 60). The v12 intersection selects the minimum Host/viewer height and FPS, and richest mutually allowed software codec (Compact, Tiles, JPEG in that order). Host-approved JPEG quality, duration and other restrictions do not change. No GPU/vendor dependency. Actual FPS may be lower due to capture, processing, pacing, network conditions and foreground/background priority.

The selected profile is immutable after the handshake and becomes the initial output and maximum recoverable adaptive geometry. v10/v11 geometry transitions and the v11 local output retargeting implementation are reused. A viewer cannot upgrade the Host's approval; a Host cannot exceed the offered viewer ceiling in the selected configuration or geometry transitions. This is a negotiated rate ceiling enforced by a conforming sender's pacing and session record budget, not a guarantee against arbitrary bursts from a malicious authenticated sender.

Example (already paired devices, explicit local Host approval still required):

```
intralink share-desktop --bind <host-address:port> --controller <paired-name> --monitor <local-id> --negotiate-profile
intralink view-desktop --device <paired-host> --negotiate-profile --max-height 720 --max-fps 30
intralink workspace --device host-1 --device host-2 --device host-3 --negotiate-profile --device-limit host-2=720:30 --device-limit host-3=540:15 --fullscreen
```

The workspace applies the global receiver ceiling to every slot; exact selected device names can override it downward. Duplicate, unknown, malformed, out-of-range or higher-than-global overrides are rejected before opening a session. Each target stores its own immutable ViewMode, used for initial connection and local reactivation. Each has independent queues, decoder state, negotiated caps, capture demand and cancellation. The common controller endpoint does not cause a shared codec or geometry. Background priority continues to reduce pacing without raising a session's approved FPS. Mixed protocol versions in one workspace remain unsupported.

## Wire exchange

ALPN `intralink/view/12-bounded`. After verified pinned identity and local approval:

1. Viewer sends the 17-byte request (16 ASCII bytes `INTRALINK/12VIEW` and one nonzero codec-capability mask, bits 0/1/2 JPEG/Tiles/Compact), followed by three bytes: height u16 big-endian, FPS u8. Unknown mask bits and unsupported limits are rejected.
2. Host independently computes the exact intersection. Sends 16 bytes: existing eight-byte approved Host VideoConfig (its mode is the richest allowed software codec), followed by eight-byte selected VideoConfig. Both configurations undergo the existing bounds checks.
3. Viewer recomputes the intersection from the advertised Host ceiling and its own offer, rejects any non-identical selected profile, and echoes all 16 bytes. Host validates exact confirmation before first capture demand. The local capture owner also verifies the selected profile does not exceed the approved Host bounds.
4. Existing adaptive records (two-byte height per changed record, complete-keyframe-only geometry transitions, record credit/order, timeout and duration), pacing and bounded decoder buffers are reused. No new remotely reachable commands or permissions.

## Validation scope

Core tests enumerate every valid receiver height/FPS and multiple Host ceilings; malformed caps/selection, independent slot configuration and CLI failures are covered. Real authenticated loopback and three independently paired simultaneous sessions exercise differing height/FPS ceilings, lossless Compact reconstruction and preserved ordering. See the current validation evidence and handoff. Physical remote performance and independently qualified hardware codecs remain separate gates.


### Optional measured receiver ceilings

`--auto-profile` with `--negotiate-profile` measures the actual local software decoder and reduces the offered receiver profile within all manual ceilings, measuring all selected decoder workers concurrently at the candidate slot layout. No wire changes or hardware identity transmission. See [RUNTIME-CAPABILITIES.md](RUNTIME-CAPABILITIES.md).
