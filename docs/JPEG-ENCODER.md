# Portable JPEG encoder optimization

The desktop encoder uses pinned `jpeg-encoder 0.7.1` (MIT or Apache-2.0, and IJG). The library is Rust and exposes optional AVX2 runtime dispatch on x86/x86_64. Application and core continue to forbid unsafe code; any SIMD implementation is in the pinned dependency. `jpeg-simd` is a weak optional feature, enabled by default, that does not pull the encoder into a non-desktop build. Non-AVX2 CPUs use scalar code. An explicit `--no-default-features` desktop build omitting `jpeg-simd` exercises the independent scalar configuration; no GPU, CUDA, native JPEG library or hardware requirement.

Production JPEG records retain the existing tag, baseline three-component JPEG, 4:4:4 chroma sampling at every quality, requested/approved quantization tables and dimensions. The historical Image JPEG encoder is retained only for diagnostic comparisons and fixture generation. The production receiver is unchanged. JPEG remains lossy; the independent quality regression compares header sampling and quantization, decoded geometry, and per-channel mean squared error on generated fixtures at qualities 35, 75 and 90, not byte-identical output. Exact Compact/Tiles records remain lossless.

The writer checks every appended byte against the existing 2 MiB maximum record size, including its tag. Allocation growth is bounded, and returned transport buffers retain capacity for reuse. A JPEG encoding failure clears the partial record and returns an error; it does not transmit a partial JPEG, silently reduce quality, retry an unsupported codec, or expand the approved profile. Adaptive JPEG policy is unchanged.

For Compact, a failed delta budget attempt proves that a complete lossless keyframe cannot fit into the same or a smaller budget, because it includes the same changed tiles plus any other tiles. Skip only that provably impossible second attempt. Run counting stops as soon as the candidate cannot beat the existing encoding or fit in its remaining budget. Preserve the smallest representation and atomic failed-budget behavior. Regression tests cover budget boundaries and the subset property.

## Reproducible local qualification

`desktop-profile-check --encoder` compares the old and new JPEG implementations using identical generated RGB canvases, quality 75, explicit 4:4:4, warm-up and 12 alternating timed encodes at 1080/720/540. The output records actual runtime AVX2 availability, payload bytes, p50, p95 and worst milliseconds; both results are decoded using the production receiver with geometry and marker checks. No screen capture, network, image recording or vault access. This is encoder-only timing, not transmission FPS. The separate `desktop-profile-check --host` measures the complete local generated-window capture/encode path and selects a bounded profile with the existing conservative 25% reserve.

Build the portable WGC variant with `cargo build --locked --release -p intralink --no-default-features --features wgc-preview,jpeg-simd,resource-monitor`. Build the portable scalar variant with `cargo build --locked --release -p intralink --no-default-features --features desktop-preview,resource-monitor`. Both use the same wire protocols and approved settings. No protocol version changes. Actual physical devices, sustained network performance, non-Windows runtime and native hardware codec integration remain separate qualification gates.


## Optional restart-strip encoding

The `jpeg-parallel` feature uses independently encoded MCU-aligned strips and assembles a single bounded baseline JPEG with restart markers. It is independently selectable from `jpeg-simd`; serial remains the default. See [JPEG-PARALLEL.md](JPEG-PARALLEL.md) for resource bounds, decoded-pixel equivalence and local stage measurements.
