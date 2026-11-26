# Transport validation — v0.21.0

## Implemented change

Correct eight existing asynchronous-test borrow errors by borrowing stable address values (`std::slice::from_ref`) or binding the address array outside `tokio::join!`. The production transport functions, relay-disabled policy, protocol v7 and diagnostic ALPN are unchanged. A new test-only dependency on the existing `intralink-vault` crate enables the integration test; there is no new runtime dependency.

`crates/intralink-transport/tests/multi_host.rs` creates a single controller and three independent host profiles. Signed offers and acceptances are confirmed separately on the hosts, and outbound route and identity grants are used to open QUIC streams. Each source emits one complete lossless Compact keyframe followed by seven changed records. The viewer uses the actual decoder and checks both each changed pixel and the complete final canvas. Two sessions are foreground and one is background. The negotiated maximum is 60 FPS; this is a finite correctness test, not a sustained-throughput measurement. No native desktop pixels or remote input are used.

## Executed on Windows

- Actual transport crate archive verified by SHA-256 before extraction; corrected source hashes also checked against the repository.
- Initial test build reported eight E0716 errors in existing tests. These were fixed; a further temporary slice in the newly added integration test was replaced before the next build.
- `cargo test --locked -p intralink-transport`: 16 unit tests passed (13.19 s), then one three-host integration test passed (1.46 s); exit 0.
- The integration verifies 24 changed frames in total, at 960x540, over three concurrent loopback connections. Each host has only its own controller grant. It uses one controller endpoint, not three separate controllers.

The subsequent combined run of `cargo clippy --locked --workspace --all-targets -- -D warnings` and `cargo test --locked --workspace` completed with exit code 0. The suite passed 65 core, 16 transport unit, one transport integration and 11 vault tests (93 total, no failures). Clippy finished in 11.48 s. The report includes exact transport source hashes and measured test durations: [transport-windows-validation.json](../benchmarks/transport-windows-validation.json).

The native GUI still uses per-session endpoints. This milestone demonstrates library capability, not a production GUI endpoint refactor. Loopback testing does not prove WAN/NAT behavior, native permissions, live revocation, GPU acceleration, or actual end-to-end 60 FPS. The complete application and platform feature gates remain open; the isolated lockfile is not the full application lockfile. No independent security audit has been performed.
