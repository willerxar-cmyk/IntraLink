# Hardware media preparation — 0.10.0

This milestone adds an optional local Linux VA-API capability/configuration probe and a platform-neutral NV12 converter. It does **not** enable H.264, VP9 or AV1 on the wire, provide a hardware encoder/decoder, or change the v6 desktop protocol. The entire application logic is Rust; the VA-API feature calls the installed native driver through the Rust `cros-libva` wrapper. No FFmpeg process or Python runtime is introduced.

## Local driver probe (Linux only)

Install Rust stable, `pkg-config`, `libclang-dev`, `libva-dev` (VA-API version 1.20.0 or newer) and the applicable driver. Debian/Ubuntu naming is used here; select matching driver packages for the actual GPU. Do not run as root just to bypass render-node permissions. Use a permitted user session. No vault, screen capture, listener, DNS update, network connection or installation of drivers is performed by the diagnostic itself.

```sh
cargo build -p intralink --features vaapi-probe --release
./target/release/intralink media-capabilities
```

The command enumerates actual character render devices under `/dev/dri`, opens each through libva, queries its vendor string and H.264/VP9/AV1 profiles and encode/decode entrypoints, and attempts to create one default VA configuration for each matching entrypoint. Low-power encoding is reported separately. A driver-advertised profile is not guaranteed to be hardware-accelerated or usable for a specific resolution/pixel format; creating a configuration is not frame encoding or decoding. No device, access errors, initialization errors and partial device queries produce a diagnostic failure, not a synthetic positive. Configuration rejection is shown per entrypoint, not silently hidden. An available driver with none of the selected profiles is a valid negative capability report. No profile is automatically selected for a remote session.

The dependency is isolated behind the Linux-only optional `vaapi-probe` feature. The default CLI and `desktop-preview` do not activate it; cross-platform builds do not require libva development files unless the feature is selected on Linux.

## RGB24 to NV12 (safe Rust)

`intralink_core::yuv::rgb24_to_nv12` converts bounded even-size RGB24 input (up to 3840x2160) into caller-owned 8-bit NV12 Y and UV planes. It implements a fixed-point approximation of BT.709 limited-range conversion with 2x2 box-filtered chroma. All dimensions, stride minima and checked buffer spans are validated before any write. Row padding remains untouched. The caller provides disjoint slices and reuses buffers; the converter itself does not allocate or spawn processes. No SIMD or GPU color conversion is claimed. This format is chroma-subsampled and not lossless; it is not a substitute for the existing exact RGB desktop codec.

At 1920x1080, tightly packed RGB24 occupies 6,220,800 bytes; NV12 occupies 3,110,400 bytes (exactly half, excluding pitch/alignment). This is an uncompressed buffer calculation, not a bandwidth or latency benchmark. Native encoders may require a different memory format, alignment, color space, chroma siting or hardware surface. Future integrations must declare color metadata and perform accurate matching rather than silently using incompatible surfaces.

The standalone Rust media harness now includes color conversion tests (black, white, grayscale, primary vectors, chroma averaging, stride padding, rejection before mutation). Run `rustc --edition=2024 --test -D warnings crates/intralink-core/src/media_harness.rs -o intralink-media-tests.exe` and then `./intralink-media-tests.exe`.

## Next native gates

| Backend | Next concrete gate |
|---|---|
| Linux VA-API (AMD/Intel and compatible drivers) | Execute the probe; verify format/size limits; allocate and upload a synthetic NV12 surface; encode/decode a bounded frame sequence and verify output. |
| NVIDIA NVENC/NVDEC | Query the installed SDK/driver and run an actual frame round-trip. |
| Windows AMD AMF / Intel QSV | Verify driver and native device initialization, supported formats and actual frame processing. |
| macOS VideoToolbox | Permissioned native capture and supported encoder/decoder session creation; verify color metadata. |
| Android MediaCodec | Explicit hardware capability and lifecycle testing on actual devices. |
| iOS VideoToolbox | Viewer first, with platform-compliant screen-sharing capabilities only. |

No backend is ready to negotiate remotely until a bounded encode/decode round-trip and malformed-input tests pass. Keep GPU dependencies optional, separate codec capabilities from permission grants, and avoid automatic downgrade or sending private desktop content during probes. For production evaluate GPU-native surfaces and zero-copy paths before optimizing CPU conversion further.

## Evidence and limitations

The current authoring runtime has no Rust toolchain, no `/dev/dri` devices, and no libva pkg-config development metadata. The dynamic libva libraries alone do not establish GPU access. No probe, Rust test, conversion benchmark or native frame round-trip has run here. The separate arithmetic reference in [nv12-reference.json](../benchmarks/nv12-reference.json) checks 140,608 uniformly sampled RGB triplets (levels 0,5,...,255), all 256 neutral grays and 100,000 deterministic random four-pixel groups. Against floating-point BT.709 limited-range with nearest rounding, the maximum observed absolute error was one 8-bit code value; grayscale chroma was exactly neutral. This validates the formula numerically, not the Rust buffer traversal, compiler output, device format or throughput. The upstream wrapper/API was checked against its public documentation; the optional dependency has not been resolved or compiled. See [VALIDATION.md](VALIDATION.md) for CI and local gates.

Upstream API references: [cros-libva 0.0.13](https://docs.rs/crate/cros-libva/0.0.13), [wrapper source](https://github.com/chromeos/cros-libva), [VA-API core](https://intel.github.io/libva/group__api__core.html).

## Windows native backend inspection — 2026-11-26

The installed machine reports an Intel Core i9-12900HK, RTX 3080 Ti Laptop GPU, and Intel Iris Xe Graphics. These are enumerated devices, not tested encoder capabilities. Cargo feature inspection confirms the default XCap backend without `wgc`. XCap 0.9.10's installed Windows source selects GDI for the default window/monitor capture. Its optional one-shot WGC adapter creates a frame pool and capture session for each call, waits up to three seconds, reads back to CPU, and attempts to disable the OS capture border. It is not a validated event-driven or zero-copy pipeline. Do not enable this adapter as an automatic performance fix. Keep visible capture indicators and explicit local consent. Evaluate a persistent, permission-preserving D3D11/DXGI/WGC adapter and real hardware encoder capability tests independently.

## Persistent capture backend selection

XCap 0.9.10's default DXGI recorder starts an unjoined thread, uses a rendezvous sender and a stop operation that only parks the worker; it is not suitable as the application session lifecycle without upstream changes. The optional one-shot WGC backend recreates the frame pool and capture session for each image. The experimental windows-capture 2.0.1 adapter uses a single session, OS-default border/cursor/secondary-window settings, bounded readback demand, one queued image, explicit stop/join, and no disk recording. It is isolated behind the Windows-only diagnostic `wgc-preview` feature and `desktop-media-check --wgc`; it accepts only the generated test child window. No production Host screen-capture backend or remote privileges are changed. It still maps a CPU-readable staging texture and copies rows; this is not GPU encoding or end-to-end zero-copy.

## Reusable D3D11 readback boundary

The optional Windows WGC diagnostic now uses `intralink-d3d11` for a retained, CPU-readable staging texture. The adapter validates RGBA8 format, device identity, source dimensions, array/mip/sample layout, row pitch and mapped span; a geometry or format change is an error rather than a silent reallocation. The mapping is scoped by an unmap guard, only initialized pixel bytes are copied, and mapped pointers never escape the adapter. The crate is the only new native FFI boundary; its unsafe operations are documented and checked by Clippy while the portable core, vault, transport and application continue to forbid unsafe code. The CPU encoder borrows the captured image. A capacity-one recycle channel returns its allocation to the WGC callback after encoding. Native staging allocations and CPU RGBA allocation/growth events are counted separately. The adapter still makes a GPU-to-CPU copy and CPU row copy; it is not a GPU encoder or a zero-copy media path. No production Host capture permission or protocol change.

## Strict D3D11 video processor scaling preview

The optional `desktop-media-check --wgc --gpu-scale` diagnostic uses `GpuScaler` in the native D3D11 crate. It requires a driver-advertised RGBA8 input/output format and full-range SDR RGB color-space conversion, and disables automatic video enhancements. A retained processor and input/output textures scale the source to an aspect-preserving output before readback; CPU letterboxing remains available for the output canvas. The raw source and readback dimensions are reported separately. Driver rejection, geometry/format changes and invalid data are errors; the requested strict GPU mode does not fall back to CPU. This is a local fixture-only diagnostic and still performs a GPU copy and CPU readback, not an end-to-end GPU codec or zero-copy solution.

## Latest-frame GPU mailbox (Windows WGC diagnostic)

The WGC handler retains one complete RGBA8 D3D11 texture (`LatestSurface`), copied from the native frame while its pool resource is valid. Every observed native update advances a checked generation; a capacity-one wake channel coalesces notifications rather than losing the latest image. The GPU video processor's input view now references that retained surface directly, removing its separate input texture/copy. The application reads only a generation newer than the last delivered generation, under the same handler mutex that serializes immediate-context use. The native device is checked before readback. Frame closure, callback failure, geometry/device mismatch and explicit cancellation are errors. An initial capture has a 2-second wait limit; later polls have a 17 ms wait budget and pump local checks at up to 8 ms intervals during waiting. No event is not proof of unchanged screen content and is never fabricated as a new native frame. Native Map/CopyResource/stop may still block and are not certified cancellation deadlines. GPU-to-GPU retention now occurs on native updates even without a consumer; quantify its idle GPU cost before production integration. No CPU readback occurs merely because of a callback. The adapter remains restricted to the generated local fixture; no production Host WGC integration or remote permission changes.

## Native ownership and context synchronization

The WGC latest-surface constructor now requires ID3D11Multithread protection on its session-owned immediate context, then verifies the protection is enabled. The handler mutex continues to serialize the complete update/consume transactions. Retention and readback query GetDeviceRemovedReason before operations and after CopyResource / Map / Unmap as appropriate; failed reads are never published by the caller. Readback validates that the supplied immediate context and texture both belong to the expected device before changing staging state. Reference: [Microsoft ID3D11Multithread](https://learn.microsoft.com/en-us/windows/win32/api/d3d11_4/nn-d3d11_4-id3d11multithread). Native per-call synchronization incurs some overhead and must be benchmarked.

The optional `desktop-geometry-check` diagnostic creates two real independent hardware devices with initialized synthetic RGBA textures, verifies a real round-trip and checks rejection of mismatched context/device/texture and changed staging dimensions. It then resizes only its own disposable child window after an explicit pipe request, observes the WGC callback failure and tests cleanup. This is not a real adapter removal, display mode change, driver reset or remote Host validation. No device is disabled or reset.

## Persistent WGC full-display source (explicit preview)

The same native mailbox, protected immediate context, retained staging resources and optional strict D3D11 scaling now serve the explicitly selected Windows Host backend. Monitor mapping uses a unique enumerated handle exactly representable as the approved XCap ID and matches the device name/geometry; the first frame must have approved dimensions. The native driver still controls capture availability. The Host ends on known device failure or prolonged absence of observed readbacks and does not switch to GDI automatically. No hardware encoder/decoder, NV12 GPU conversion, physical monitor loss or full-display streaming performance is claimed by this integration.

## Native BT.709 NV12 video processor

The native D3D11 module now shares one video-processor implementation between existing RGBA scaling and GPU-resident RGBA-to-NV12 conversion. It explicitly negotiates full-range RGB BT.709 and studio-range YCbCr BT.709 with left-sited 4:2:0 chroma. NV12 surfaces require even dimensions; texture layout, device identity, context identity, format support and driver removal are checked. A single bound output texture is retained; the borrowed NV12 surface may be consumed by a future hardware encoder before the next conversion, under the same serialized context. CPU readback is used only by the diagnostic. NV12 staging copies packed Y followed by interleaved UV, respecting native row pitch and bounds. No change to the desktop wire protocol or negotiated codecs.

`desktop-video-adapters` lists DXGI indices and adapter LUIDs. Use `desktop-nv12-check --height 1080 --samples 12 --vendor nvidia` or specify `--adapter-index` when a vendor is ambiguous. Indices are current-enumeration diagnostics, not stable stored identities. Explicit selection rejects software render adapters, ambiguity and vendor mismatch without fallback. The machine enumerates two Iris Xe entries with distinct LUIDs and one RTX 3080 Ti Laptop GPU; this does not establish two separate Intel GPUs. The native diagnostic generates changing color bands, performs RGBA/NV12/RGBA through actual video processors, validates interior YUV and RGB samples and requires one reused staging allocation per direction. It is not an H.264/HEVC encoder or decoder. It does not capture the desktop.

References: https://learn.microsoft.com/en-us/windows/win32/api/d3d11_1/nf-d3d11_1-id3d11videoprocessorenumerator1-checkvideoprocessorformatconversion ; https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format

### External hardware codec feasibility (not IntraLink integration)

Using the existing installed FFmpeg only as a diagnostic with generated 1080p60 H.264, NVENC encoding and explicitly selected CUDA/CUVID decoding completed 60/60 frames with 60 distinct decoded hashes. Intel QSV with an explicitly initialized hardware session on D3D11VA adapter index 0 and CPU NV12 input also completed 60/60 frames. The explicitly GPU-uploaded Intel path failed with D3D11 texture creation HRESULT 0x80070057; adapter index 2 failed MFX session creation (-9). Preserve those failures. Do not silently switch paths or label the CPU-input test zero-copy. The result validates hardware feasibility, not Rust encoder/decoder integration, performance under sustained load, or cross-adapter interoperability. Raw external diagnostic evidence: benchmarks/hardware-codec-feasibility-windows-20261126.json. Prioritize the native NVENC/NVDEC path, then oneVPL QSV with exact adapter selection and actual surface registration/allocation tests.

Reproduce native conversion on the current Windows machine (indices are diagnostic and must be enumerated again after device changes):

```powershell
intralink desktop-video-adapters
intralink desktop-nv12-check --height 1080 --samples 60 --vendor nvidia --adapter-index 1
intralink desktop-nv12-check --height 1080 --samples 60 --vendor intel --adapter-index 0
```

For native NVIDIA encoding, investigate the SDK DirectX session and explicit registration of the retained NV12 texture; never overwrite a registered input until encode completion. Keep decoder/output surface ownership and cross-adapter copies explicit. For QSV, distinguish the verified hardware CPU-input session from the failed D3D11 texture-upload path; test resource flags, alignment, oneVPL surface allocation and exact adapter selection before enabling GPU-resident encoding. Neither path is yet implemented in the Rust transport.

## Native NVIDIA H.264 encoder preview

The opt-in Windows `nvenc-preview` feature builds a native DirectX H.264 encoder using pinned mini-nvcodec 0.1.0 C ABI declarations (SDK 13.1) and libloading 0.9.0. The Cargo lock includes mini-cuda 0.1.1 transitively. The implementation uses its own System32-restricted library loader instead of the dependency's default DLL search, queries the driver's maximum API version, requires SDK 13.1, and rejects missing driver functions. No software or cross-adapter fallback. Existing builds do not load the NVIDIA library unless the diagnostic is invoked.

The encoder checks NVIDIA vendor and exact NV12 texture/device, uses one retained registered input resource and one synchronous bitstream buffer, and requires an output with the matching input timestamp for each accepted picture. P1 ultra-low-latency, bounded CBR, no B-frame reorder or lookahead, periodic and requested IDR with SPS/PPS, BT.709 limited-range metadata. Output is bounded to 2 MiB per access unit. After any failed submission no new pictures are accepted; cleanup unlocks, unmaps, unregisters and destroys owned resources before releasing COM references and DLL. Synchronous native operations may block; this diagnostic is not a cancellation-safe production encoder. NVDEC and remote codec negotiation remain separate gates.

The command `desktop-nvenc-check --height 1080 --samples 60 --adapter-index 1` exercises a generated RGBA upload, GPU NV12 conversion, native NVENC registration and encoding without CPU pixel readback. Optional `--output` writes a bounded synthetic Annex B clip to a new file, never overwrites. The current driver and format support must be tested before enabling any actual streaming use. See https://developer.nvidia.com/video-codec-sdk and https://docs.rs/mini-nvcodec/0.1.0/mini_nvcodec/.

### Native NVDEC diagnostic continuation

The same optional feature now includes native CUVID parsing/decoding, loaded through the system-only DLL loader. A private CUDA context is created only when no CUDA context is current on the diagnostic thread, on the exact NVIDIA adapter matched by DXGI/CUDA LUID. The decoder requires H.264 progressive 8-bit 4:2:0 with BT.709 limited range, exact display/coded geometry and bounded decode surfaces. It uses only generated access units, validates native decode status, mapped pitch, timestamp order and every sampled color band. Six mapped rows are copied to CPU per frame for validation; no full decoded pixel surface is copied or presented. Parser callbacks are synchronous, errors are retained and Rust panics are contained before the C boundary. Actual remote/malformed stream qualification and CUDA-to-presenter surface lifetime/interoperability remain pending.

During native encoder bring-up, both encode and lock API returns succeeded and the timestamp matched, but the raw unsigned hwEncodeStatus field was 2. The SDK header does not specify an NVENCSTATUS mapping for this field; the diagnostic records its value per frame and independently validates the output, rather than silently discarding or equating it to API success. This unresolved field interpretation remains a production-review item. No driver errors are ignored. External ffprobe/CUVID independently decoded the native 540p/1080p H.264 clips with correct BT.709 metadata, no B-frames, and sampled error at most one. The Rust native decoder then verified 6/6 540p and 180/180 1080p generated frames. No FFmpeg dependency is present in the Rust application. Use `desktop-nvenc-check --height 1080 --samples 180 --adapter-index 1 --decode` to exercise both native codecs, never a user's screen.

## Explicit-adapter native WGC and NVIDIA pipeline (development)

The new generated-window-only WGC diagnostic constructs a free-threaded Direct3D11CaptureFramePool from the explicitly selected NVIDIA D3D11 device. It keeps the default native capture border/cursor policy, enumerates only the spawned test child by PID/title, requires exact texture ownership and initial geometry, and ends on changed geometry, source closure, invalid timestamp, wait failure or device removal. Native callbacks only coalesce wake notifications; the caller thread owns WinRT and GPU transactions. Retains one stable RGBA surface and one NV12 processor output; directly registers that NV12 texture with NVENC on the same D3D11 device. NVDEC matches the DXGI LUID. OS compositor-internal adapter transfer is not measured. No general desktop monitor or network codec privilege was introduced. The normal Host remains v8.

Debug generated-window native 540p/720p/1080p passed 60/60/180 frames, maximum sampled YUV error 0/0/1, exact adapter NVIDIA RTX 3080 Ti Laptop. 1080p source 3336x1878, output 1920x1080, local capture+conversion+encode+decode 54.96 FPS including startup, capture p50/p95 12.128/14.800 ms, convert+encode 2.345/2.589 ms, decoder submit 2.191/2.431 ms. These are local synthetic window results, not an end-to-end network or physical monitor test.

Optional local GPU presentation development uses CUDA-D3D11 registered R8 and R8G8 textures, two synchronous device-to-array plane copies, explicit unmap ordering, a static BT.709 limited-range shader and a private windowed DXGI flip swap chain. NV12 is not directly registered and this is GPU-to-GPU copying, not zero-copy. Four 1x1 RGB samples (16 bytes) per submitted frame are read back for validation in addition to existing diagnostic YUV samples. No arbitrary remote bitstream accepted. References: https://docs.nvidia.com/cuda/cuda-driver-api/cuda_driver_api/group__CUDA__D3D11.html ; https://docs.nvidia.com/cuda/cuda-driver-api/cuda_driver_api/group__CUDA__GRAPHICS.html ; https://learn.microsoft.com/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgifactory2-createswapchainforhwnd . The presentation gate is independent from the capture+codec gate.

### Native GPU presentation and driver teardown

The optional --present path registers the R8 luma and R8G8 chroma textures on the current private NVIDIA CUDA context and copies both mapped NVDEC planes directly to the D3D11 arrays. After unmap ordering, the D3D11 shader draws to an owned visible two-buffer windowed flip-discard DXGI swap chain. Actual Present(SyncInterval=0) S_OK submissions and sampled RGB back-buffer pixels are counted, separately from NVDEC display callbacks; this is not a monitor scan-out timing proof. The preview window can be closed locally or stopped with Escape while focused. Renderer reads only 16 RGB diagnostic bytes per frame plus the existing NVDEC YUV row samples, not complete CPU frames. The path uses GPU-to-GPU copies, not zero-copy. The native calls are synchronous and can block; arbitrary network AVC is not enabled.

Initial presentation tests validated rendered colors but failed during process teardown with Windows application fault 0xc0000005 in nvdlistx.dll_unloaded (NVIDIA driver 610.60). Retaining only the fixed system NVIDIA DLLs in process-scoped OnceLock handles and stopping WGC before codec teardown did not alone fix the fault. The successful revision explicitly releases swap-chain buffer/view references and flushes deferred D3D11 destruction before closing the owned HWND; retains WinRT initialization until all selected capture/D3D11 objects are released; explicitly drops the device and apartment before returning. CUDA registrations, contexts, codecs and GPU objects are still destroyed per session; no driver reset, process-exit bypass or error suppression. The driver module handles alone are process-lifetime. Python verifies actual executable exit codes as well as content and teardown markers. Final Debug repeated tests: 60/60 540p, 60/60 720p, 180/180 1080p twice, all actual exit 0; the matched process IDs had no corresponding new application crash event in the subsequent log check. 1080p local complete synthetic capture/codec/present at 55.71 and 55.60 FPS, sampled RGB max error 3, presentation median 0.776 and 0.729 ms. Driver internals were not proven as root cause; other drivers/displays and actual physical sessions still need qualification.


### Headless NVDEC under the reduced token (2026-11-26)

The new developer example `restricted_decoder_probe --decode-luid HIGH:LOW` reuses the actual native parser/decoder and exact CUDA LUID match, without creating the optional D3D11 presenter. On the current RTX 3080 Ti Laptop LUID 00000000:00017777, Debug decoded six embedded native-generated 1920x1080 access units immediately (6/6/6, max sampled YUV error 1, map/sample p50 1.808 ms, child exit 0). No full-frame CPU readback; no new GPU sharing mechanism or physical desktop stream. Existing windowed presentation remains separate; shared resource handles and synchronization are the next native integration gate.


### Optional acceleration policy (2026-11-26)

CUDA/NVENC/NVDEC are optional NVIDIA-specific capabilities. Normal sharing and decoding use the required CPU baseline, `desktop-preview` with Compact/Tiles/JPEG, and neither NVIDIA library is present in that feature graph. Capture and scaling may use native OS APIs independently of the codec; hardware-backed WGC is not a CUDA requirement. Other vendor/OS codecs must be independently qualified (D3D11/Media Foundation or oneVPL, AMD AMF/VA-API, Apple VideoToolbox, Android MediaCodec). Negotiate only verified compatible codec profiles; preserve a software path and do not silently substitute an unsafe decoder or weaken a requested isolation policy. See `ARCHITECTURE.md` and `DESKTOP.md`.


### Runtime capability inventory and measured receiver profile

`desktop-profile-check` uses generated JPEG/Compact/Tiles data and the actual software decoder. `--slots` applies a conservative serial decode budget; `--auto-profile` on a v12 viewer/workspace limits its offer to measured local capability. `desktop-profile-check --hardware` on Windows WGC enumerates exact DXGI adapter handles, confirms created-device LUID, queries driver-advertised NV12 decoder configurations and separately verifies actual generated 1080p NV12 conversion. No arbitrary native bitstream or remote hardware codec enabled. CPU baseline is independent of this diagnostic. See [RUNTIME-CAPABILITIES.md](RUNTIME-CAPABILITIES.md).
