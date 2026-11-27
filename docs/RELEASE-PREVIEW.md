# Public preview release policy

The project publishes development previews as GitHub pre-releases, not stable production packages. The initial Windows x64 package is derived from the qualified staged-readback checkpoint `de420ad` (code `507afc0`). The only changes between that checkpoint and the public packaging commit are documentation and release tooling. No rebuild is required for an unchanged binary input.

- Binary SHA-256: `989a1fdb2a8181cc5a7784dcad32ad9e2f2902d01e3c8dd729aafd2cfdb83b8f`.
- Windows x64 build features: `wgc-preview,jpeg-simd,jpeg-parallel,resource-monitor` without default features. No NVIDIA codec dependencies.
- Automated gates: 98 standalone, 219 CPU workspace, 232 NVIDIA workspace, 61 scalar app tests; four strict Clippy configurations. Actual native geometry, retarget, pipeline, visual and staged diagnostic passes; see APP-VALIDATION.md and benchmarks.
- The portable archive contains `intralink.exe`, README, MPL-2.0 license, resolved dependency inventory and available upstream license/notice files. The source archive is a Git archive of the release tag, including Cargo.lock. Verify CRC, archive commit, binary hash and manifest hashes before uploading.
- Initial preview distribution is unsigned and not production-certified. No physical multi-device qualification, sustained 60 FPS, remote input, unattended access or macOS/mobile support is claimed. Third-party inventory is not an independent license or security audit.

Use a unique new version tag for each public release. Never overwrite an existing asset or tag; publish subsequent versions as pre-releases until the production gates pass. User explicitly authorized public GitHub release publication on 2026-11-27; use only non-force fast-forward pushes, inspect the remote before every push, and preserve all prior local artifacts. Do not publish secrets, private vaults, private device data, or unreleased unvalidated binaries.
