# Local runtime capability calibration

Status: opt-in software receiver calibration and local native capability inventory. No new media wire protocol or permission. v12 bounded negotiation is reused; CPU remains the required media baseline.

## Software receiver calibration

`desktop-profile-check --slots 1` measures the real software Decoder on generated JPEG (quality 90), complete Compact lossless keyframes, and absolute Tiles/Compact updates. Each allowed output tier (1080/720/540) uses one warm-up and six timed decodes per allowed representation; local fixture generation is excluded from timing. Check decoded geometry and marker pixels, and the complete lossless keyframe. Use the worst observed decode time across these fixtures, not nominal CPU clock, GPU vendor, hardware thread count, or advertised codec. These patterns are representative, not a proof of worst-case hostile JPEG or sustained rendering capacity.

The local policy reserves 25% of one serial decoder time budget for other work and divides the remaining budget over the number of selected slots (1..8). For each measured tier, the recommended FPS is floor(750 ms / (worst decode time * slots)), bounded by the user's requested ceiling. Prefer the highest permitted resolution able to meet min(30, requested FPS); otherwise choose the fastest feasible tier, breaking ties toward greater resolution. Reject invalid, zero or insufficient observations rather than pretending the target was qualified. This conservative serial budget intentionally does not assume multicore scaling or guaranteed network capacity. Actual concurrent worker throughput and resource pressure remain separate measurements.

In the v12 receiver and workspace, `--auto-profile` explicitly opts in to calibration before starting any network session. All manual ceilings are retained as upper bounds, including per-device overrides. A single calibration covers the selected codec ceiling and all lower software modes and eligible geometries; the workspace shares the resulting conservative budget across its selected slots. Only the selected receiver height/FPS is sent through existing v12 negotiation. Raw hardware identities, timings and CPU counts stay local. Calibration never increases a limit or modifies a saved grant. A failed calibration ends the requested automatic start with an explicit error; the existing manual profile remains available. The Host still approves its own maximum and uses existing adaptive encoding/geometry control; automatic Host capture/encoder selection is not introduced here.

Examples:

```
intralink desktop-profile-check --slots 1
intralink desktop-profile-check --slots 3
intralink view-desktop --device paired-host --negotiate-profile --auto-profile
intralink workspace --device host-1 --device host-2 --negotiate-profile --auto-profile --device-limit host-2=720:30
```

## Optional native Windows capability inventory

With a `wgc-preview` build, `desktop-profile-check --hardware` additionally enumerates the real DXGI adapters (bounded to 64). Skip software adapters. Create each D3D11 device from the exact current enumerated adapter handle and verify its LUID. Query driver-advertised H.264, HEVC Main, VP9 Profile 0 and AV1 Profile 0 decode support, NV12 output support, and configuration counts at 540p/720p/1080p. Unknown, unavailable and failed operations are reported distinctly; no vendor-name inference. The query does not instantiate or submit input to a native decoder.

Independently execute the actual native generated-color RGBA→NV12→RGBA conversion on each successfully created hardware device at 1080p, six changing samples, with bounded textures, staging reuse and checked color errors. This demonstrates conversion on that device, not hardware encoder/decoder support or remote codec safety. A missing/unsupported GPU is an inventory result, not a required software backend. CUDA/NVENC are not dependencies of the WGC capability build. No real screen capture, driver reset, network access or arbitrary AVC input. The hardware flag is rejected on a CPU-only build or a platform without this adapter. Existing Linux VA-API inventory remains separate and requires actual Linux qualification; Apple/Android adapters and portable hardware codec integration are still pending.

All native decoder configurations remain diagnostic only, with zero remote-enabled hardware codecs. Never advertise a remote native codec based on driver enumeration or a color conversion success. The existing NVIDIA restricted fixture, isolation gates and explicit capture backend policy remain unchanged.
