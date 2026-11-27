# Upstream references and licensing

- DWService agent: https://github.com/dwservice/agent — public repository, core MPL-2.0, individual app/library licenses may differ. Reference folders include `core`, `app_desktop`, `app_filesystem`, `app_resource`, `app_shell`, `app_texteditor`, `app_logwatch`, native capture and OS-specific packages. Reference functionality: https://www.dwservice.net/en/applications.html
- DuckDNS official API: https://www.duckdns.org/spec.jsp ; FAQ: https://www.duckdns.org/faqs.jsp
- Iroh: https://docs.rs/iroh/latest/iroh/ ; builder: https://docs.rs/iroh/latest/iroh/endpoint/struct.Builder.html ; endpoint identity and addresses: https://docs.rs/iroh/latest/iroh/struct.EndpointAddr.html
- Iroh persistent secret key: https://docs.rs/iroh/latest/iroh/struct.SecretKey.html ; BindOpts: https://docs.rs/iroh/latest/iroh/endpoint/struct.BindOpts.html
- RustCrypto: https://docs.rs/argon2/0.5.3/argon2/ ; https://docs.rs/chacha20poly1305/0.10.1/chacha20poly1305/
- Staged writes: https://docs.rs/tempfile/latest/tempfile/struct.NamedTempFile.html ; local advisory locks: https://docs.rs/fs2/latest/fs2/trait.FileExt.html
- Iroh signatures: https://docs.rs/iroh/latest/iroh/struct.Signature.html ; https://docs.rs/iroh/latest/iroh/struct.PublicKey.html
- DNS-SD: https://docs.rs/crate/mdns-sd/0.21.7 ; RFC 6762 and RFC 6763; shared IPv4 space: https://www.rfc-editor.org/rfc/rfc6598
- QR encoding: https://docs.rs/qrcode/0.14.1/qrcode/
- Tauri 2: https://v2.tauri.app/
- Rust targets: https://doc.rust-lang.org/stable/rustc/platform-support.html
- Android screen sharing: https://developer.android.com/media/grow/media-projection
- Google Play AccessibilityService: https://support.google.com/googleplay/android-developer/answer/10964491
- Apple screen capture: https://developer.apple.com/documentation/ScreenCaptureKit

This source is an independent implementation, not a fork or a port of copied DWService code. MPL-2.0 is a proposed project license; confirm the final project decision before external publication. Review every bundled library license, codec patent obligations and platform distribution terms. Produce an actual third-party notice inventory from the resolved dependency graph and an SBOM before shipping. Use the IntraLink name only after appropriate trademark and package-name checks.

## Experimental desktop dependencies

- XCap 0.9.10, native capture: https://docs.rs/crate/xcap/0.9.10 (Apache-2.0). Linux build requirements are upstream-specific.
- image 0.25.10, JPEG: https://docs.rs/image/0.25.10/image/ (MIT OR Apache-2.0).
- MiniFB 0.29.0, prototype display: https://docs.rs/crate/minifb/0.29.0 (MIT OR Apache-2.0; verify package metadata before release).
- These versions are provisional; resolve and audit the actual complete dependency tree, including capture and JPEG transitive dependencies, before release.

## Optional native media diagnostics

- cros-libva 0.0.13: https://docs.rs/crate/cros-libva/0.0.13 and https://github.com/chromeos/cros-libva (BSD-3-Clause per upstream repository; verify resolved package). Native VA-API: https://intel.github.io/libva/group__api__core.html. Optional Linux only, development headers and installed driver required.

## v0.11 resource sampling

- [sysinfo 0.38.2 documentation](https://docs.rs/crate/sysinfo/0.38.2) — native resource APIs and selective refresh guidance.
- [sysinfo migration guide](https://github.com/GuillaumeGomez/sysinfo/blob/main/migration_guide.md) — global CPU usage API.

## Optional Windows NVIDIA codec diagnostics (v0.22.0)

- NVIDIA Video Codec SDK: https://developer.nvidia.com/video-codec-sdk ; programming guides: https://docs.nvidia.com/video-technologies/video-codec-sdk/
- `mini-nvcodec` 0.1.0 and `mini-cuda` 0.1.1: https://github.com/AdrianEddy/wgpu-interop — both MIT OR Apache-2.0 per inspected local registry manifests. The optional diagnostic uses their C ABI types, not their default DLL loaders.
- `libloading` 0.9.0: https://github.com/nagisa/rust_libloading/ — ISC per inspected local registry manifest; system-directory-only Windows loading is required in the local driver wrapper.
- No NVIDIA driver DLLs or CUDA SDK redistributables are bundled. Driver compatibility and commercial H.264 distribution/patent licensing review remain separate release gates. The optional FFmpeg verification script uses an externally installed executable only.


## Software KVM references (2026-11-27)

- Deskflow: https://github.com/deskflow/deskflow — GPL-2.0 project, reference for multi-computer keyboard/mouse sharing and screen layout. No source copied or linked.
- OpenMouse mouse-protocol: https://github.com/OpenMouse-Project/mouse-protocol — AGPL-3.0; reference for device-level input protocol ideas, not a network KVM transport dependency. No source copied or linked.
- Mousemaster: https://github.com/petoncle/mousemaster — reference for keyboard-driven pointer control and configurable input modes. No source copied or linked; no license compatibility assumed.

The release uses the existing checked-in MPL-2.0 LICENSE and independent Rust implementation. `tools/release_inventory.py` inventories the resolved Windows x64 portable normal/build dependency graph and includes original available license/notice files. Exact upstream Git commits supply omitted root notices where possible. This inventory is not a completed legal or security audit, and no NVIDIA driver DLLs are distributed.
