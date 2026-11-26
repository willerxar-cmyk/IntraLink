# Local resource monitoring (v0.11)

This optional, foreground command is the first foundation of the resource module. It samples aggregate CPU usage, logical CPU count, total RAM, available RAM and used RAM (total minus available) from the current OS. No process names, file paths, serial numbers, user identities, network interfaces, hostnames or other device inventory are collected. Nothing is transmitted, persisted or made accessible to peers. It neither opens nor reads an IntraVault. The command is not a remote management implementation.

## Build and use (after resolving the lockfile)

```sh
cargo run --locked -p intralink --features resource-monitor -- resources
cargo run --locked -p intralink --features resource-monitor -- resources --samples 10 --interval-ms 1000 --json
```

The interval is 500–10000 ms and the count is 1–120; the default is one sample after a 1000 ms interval. The OS-specific minimum CPU update interval is honored. The sample interval recorded in output is the actual monotonic time between completion of the CPU refreshes, including any scheduling overhead. Sampling is sequential and reuses one native system object. No queue or historical sample buffer is held. Slow output can extend the sampling interval. Ctrl+C interrupts the foreground command normally.

`--json` emits a JSON object per line, with schema:

- `sequence`: positive sample ordinal, starts at 1.
- `interval_ms`: measured elapsed interval in milliseconds.
- `logical_processors`, `cpu_usage_percent`: validated aggregate CPU values or `null`.
- `memory_total_bytes`, `memory_available_bytes`, `memory_used_bytes`: validated bytes or `null`.

Unavailable values must not be interpreted as zero. The command fails if the OS is unsupported or neither metric is valid. OS-reported availability and usage can differ from other tools, especially in containers; these are not cgroup-specific metrics. Native backend limitations still apply. No GPU, swap, storage, process or temperature metrics yet. Mobile targets remain unverified.

The optional `sysinfo` dependency uses only its `system` feature with default features disabled. No external monitoring process is invoked. Actual usage, memory overhead and sample timing still require real Rust execution and comparison on Windows, Linux and macOS. Future remote resource viewing must be a separate, permission-scoped operation with explicit consent and revocation. See [VALIDATION.md](VALIDATION.md).
