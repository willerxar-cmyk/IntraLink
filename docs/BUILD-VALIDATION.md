# Reproducible validation from a Rust-equipped machine

The source archive does not contain a fabricated Cargo.lock or build artifacts. Install the official Rust stable toolchain with Cargo, rustfmt and Clippy on a development machine. Resolve the first dependency graph, review it and commit Cargo.lock. Existing optional native desktop/VA-API prerequisites are documented in [DESKTOP.md](DESKTOP.md) and [HARDWARE.md](HARDWARE.md). Review dependencies before executing. No remote operation or privileged service is started by the validation runner.

From the extracted repository root:

```sh
rustc --edition=2024 -D warnings tools/validate.rs -o intralink-validate.exe
./intralink-validate.exe --resolve --resources
```

The executable extension works on Unix and Windows; PowerShell may also use `.\intralink-validate.exe`. For Windows/Linux add `--desktop` after installing native build requirements. Add `--vaapi` on Linux only with libva development dependencies. Omit `--resolve` after committing the reviewed Cargo.lock. The runner does not automatically install dependencies, alter rustfmt configuration or overwrite an existing lockfile. Cargo may fetch dependencies required by its normal operations.

The runner checks compiler and Cargo versions, compiles and executes the actual registry-independent Rust test harness, checks formatting, resolves only a missing lockfile when explicitly allowed, checks locked dependency metadata, runs workspace Clippy with `-D warnings`, tests and release builds. Requested optional feature gates run independently. No stale test executable is run if compilation fails. A failure in formatting or Clippy does not hide subsequent diagnostics. Any failed or blocked gate returns a nonzero exit. Preserve terminal output for review; redirect it to a local log if desired.

Once local automated checks succeed, execute the security, native consent, direct-network, GPU, cancellation, throughput and platform tests in [VALIDATION.md](VALIDATION.md). The runner does not establish device compatibility, audit cryptography, test real capture or prove a GPU encoder works. Hosted CI is defined, not executed by this source archive.

When multiple supported feature flags are requested, the runner also checks them together. CI adds combined Windows desktop/resource and Linux desktop/resource/VA-API builds, tests and Clippy gates. The individual feature gates remain in place to isolate failures. No CI run or lockfile resolution was performed here.

### Isolate build outputs on a shared development machine

PowerShell: set `$env:CARGO_TARGET_DIR = Join-Path (Get-Location) "target"` before running Cargo or the validation runner. This avoids waiting on another project using a globally configured target directory. The v0.19 Windows validation was limited to an isolated workspace containing `intralink-core` and the actual overview renderer used by the portable harness. Its dependency resolution cannot substitute for the complete workspace lockfile.


### Mandatory independent software media gate (2026-11-26)

Run the Windows/Linux `desktop-preview,resource-monitor` build, Clippy with `-D warnings` and workspace tests without `nvenc-preview`; inspect `cargo tree --locked -p intralink --no-default-features --features desktop-preview,resource-monitor -e normal` to ensure no mini-cuda or mini-nvcodec dependencies. On Windows execute the actual generated-window `desktop-media-check --height 1080 --samples 60 --codec compact` and lower-resolution alternative with the CPU-only codec executable. The native OS capture path may still use platform graphics APIs; no NVIDIA codec runtime is required. Preserve a separately named portable Release artifact, and validate optional accelerated builds separately. The dependency-free media harness now includes the H.264 protocol and portable quality policy modules; the local overview banner uses a standard-library error instead of a registry dependency, preserving actual source coverage.
