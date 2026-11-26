# Windows portable core validation — v0.19

## Scope

On 2026-11-26, the actual portable Rust source files from v0.18 were copied to `D:\IntraLink-validation-v019\IntraLink` on an authorized Windows x64 development machine. SHA-256 hashes matched all portable modules, the original standalone harness, the overview renderer and both codec examples. Only this subset of the workspace is present there. An isolated root manifest includes the original `intralink-core` crate and its serde dependency; it retains version 0.18.0 for the comparison runs. The packaged v0.19 version bump is metadata only. The isolated Cargo resolution is not the full project resolution, and its Cargo.lock is not distributed as the full workspace lockfile.

The pre-change portable harness and Cargo library tests each passed 64/64. They overlap; do not sum the test counts. Clippy initially failed on `collapsible_if` in Compact palette selection. The actual diagnostic was fixed without suppressing lints. In addition, the solid tile check now validates the first RGB row and compares the remaining complete rows, avoiding per-pixel RGB comparisons of uniform regions. A regression test covers a partial edge tile with different uniform rows. No codec tags, wire layouts, authorization, vault schema or dependency changes.

## Final candidate gates

- `rustc --edition=2024 --test -D warnings crates/intralink-core/src/media_harness.rs -o intralink-media-tests.exe`: 65 passed, zero failed, including the local overview renderer.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed **in the isolated core-only workspace**, including both examples.
- `cargo test --locked --workspace`: 65 passed, zero failed, including three library-level role/probe/DDNS tests instead of the three renderer tests.
- `cargo build --locked --release --examples`: passed in the isolated core-only workspace.
- Both actual optimized example programs executed successfully. No full workspace `cargo fmt`, build, Clippy, or test gate is certified.

The machine has a globally configured shared Cargo target directory. The initial release build waited on another project's build lock; only our own pending command was terminated. Subsequent commands used `CARGO_TARGET_DIR=D:\IntraLink-validation-v019\target`.

## Benchmarks

The raw numeric observations are preserved in [before tiles](../benchmarks/codec-windows-before-tiles.csv), [before keyframes](../benchmarks/codec-windows-before-keyframes.csv), [after tiles](../benchmarks/codec-windows-after-tiles.csv), and [after keyframes](../benchmarks/codec-windows-after-keyframes.csv). Both builds produced the same payload sizes and acceptance results for the deterministic scenes. The separate-run timings are confounded by variable host load: the unchanged code paths also became much slower. They are not a reliable before/after speedup estimate.

Use the [interleaved comparison](../benchmarks/codec-windows-interleaved.csv) instead: 41 paired timed iterations per 1080p scene in one optimized Rust process, with old/new order alternated, reusable outputs, warm-up, exact byte equality checked on every iteration and pixel reconstruction checked for accepted keyframes. The uniform keyframe median decreased from 1453.60 to 421.30 microseconds (about 71%). The UI-like fixture remained approximately 5.6 ms; no meaningful gain is claimed for it. The high-entropy fixture exceeded the lossless attempt budget in both versions and returned no partial payload. These are CPU encoder microbenchmarks, not live capture, GPU, network or FPS measurements. One synthetic paired run does not certify performance across workloads or devices.

### Reproduce the paired test

Extract v0.18 and v0.19 into distinct directories. In an empty temporary benchmark directory, copy the v0.19 `tools/codec_ab.rs` and `crates/intralink-core/src/desktop.rs` to `codec_ab.rs` and `desktop.rs`. Copy v0.18 `crates/intralink-core/src/media.rs` to `old.rs` and its `media/compact.rs` to `old/compact.rs`. Copy the corresponding v0.19 files to `new.rs` and `new/compact.rs`. Compile from that directory:

```sh
rustc --edition=2024 -D warnings -O codec_ab.rs -o codec-ab.exe
./codec-ab.exe
```

The original benchmark sources remain in `crates/intralink-core/examples/`; run those with `cargo run --release -p intralink-core --example tile_benchmark` and `cargo run --release -p intralink-core --example keyframe_benchmark` after dependency resolution. Use a quiet machine, record compiler/CPU/OS, repeat several times and compare equivalent payloads/fidelity.

## Open release gates

Transfer the complete source archive to Windows, not just the isolated core. Extract separately from other projects. Resolve/review/commit the actual workspace lockfile; execute the full validation runner including desktop and resource features. Verify the vault, direct authenticated QUIC, actual capture and display, cancellation, three active hosts, native fullscreen, sustained 1080p60, and platform permissions. Native GPU encode/decode, consented input, remote monitor selection, NAT traversal, audio, file manager, native secure key storage, mobile UI and packaging are still open. Do not ship or claim a complete remote-control product on these core-only results.
