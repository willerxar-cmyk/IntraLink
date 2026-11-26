# Architecture / ADR-001

Status: v0.9.0 source extends the read-only desktop session while retaining direct-only diagnosis, vault, signed invitations and opt-in LAN discovery. No Rust build/test is confirmed.

## Product model

One product and common protocol. Controller stores its own authorized device inventory. Host stores its own identity and independent controller ACL, not the controller's private key, vault, inventory or provider credentials. Hybrid holds both roles in separate compartments. A host cannot initiate an administrative session into its controller unless the controller independently chooses host mode and grants that identity access. Replies on a controller-initiated session are not permission to initiate a new session. No mandatory IntraLink account, directory, cloud database, analytics, relay or subscription.

## Trust and addressing

The peer public key is the durable identity; IP addresses, DNS names, ports and LAN discovery are changeable routing hints. An imported device record must pin the peer identity; a DNS change never rotates trust. A controller vault contains per-device records, scopes and local labels; private identity keys stay in the local passphrase-encrypted vault in v0.9; native platform secure storage is planned. Offline recovery is a user-controlled encrypted export with format version and KDF parameters. Current pairing requires local consent and independently verified peer identities, not trust-on-first-use via DNS. Expiring invitations are implemented in source with signed offline offer and acceptance, consumed on local host confirmation. See [ENROLLMENT.md](ENROLLMENT.md). A QR code carries the public enrollment document, never a private key or password.

## DDNS

A DuckDNS label must already exist. Its documented API updates A/AAAA and TXT records; there is no documented create-label API. Do not automate signup, scrape the website, or circumvent provider limits. v0.3 supports a manually prompted one-shot update of A/AAAA records, including explicit dual-stack updates, and an optional foreground IPv4 refresh interval. Later support provider adapters (DuckDNS, RFC2136 or user's DNS API) with scoped record credentials where supported. DuckDNS tokens are account-wide; keep them in owner-controlled secure storage and do not distribute a shared token to all hosts. Prefer owner router DDNS, separate owner-administered provider accounts or another provider supporting per-record scopes. If the controller updates a DNS record for a host, obtain the host IP through an already authenticated channel or explicit owner input; never mistake the controller's detected IPv4 for the host's address. Multiple devices may share a public IPv4; DNS alone cannot distinguish their services. Use explicit per-host reachable ports or distinct reachable IPv6 addresses. Do not publish device names, inventory or private material as DNS metadata.

## Connectivity policy

1. LAN address candidates, optionally local discovery after opt-in.
2. Routable IPv6 with owner-approved firewall access.
3. DDNS/public IPv4 with an owner-administered reachable mapping.
4. Authenticated candidate exchange and NAT traversal, only with an explicitly configured signaling path. STUN/discovery is not data relay. It cannot guarantee reachability, especially with restrictive CGNAT or symmetric NAT.
5. Optional owner-operated personal relay or approved existing VPN, visibly labeled and off by default. A relay forwards encrypted data without receiving endpoint private keys. Availability and hosting costs depend on the owner. No mandatory public infrastructure.

The diagnostic implements path 1/2/3 when an address is reachable, not path 4/5. With explicit flags, IPv4 LAN mDNS provides untrusted candidate addresses for the pinned peer; it runs only during the foreground diagnostic. On LAN failure, the controller tries the saved route. It uses Iroh's Minimal preset, disables relay and address lookup, omits default portmapper feature and binds one explicitly selected host socket or controller sockets for the resolved address families. No default relay/discovery services are configured. DNS resolution uses the operating system. The controller supplies up to four candidates per family to Iroh; path choice and zero unexpected egress must still be tested. The controller can make up to three attempts, performing fresh resolution each time; DNS caches may still delay changes. This is not migration or automatic reconnection of an active media stream. Continuous path monitoring is planned. Do not enable automatic firewall/UPnP configuration without explicit user opt-in and a removal path.

## Proposed workspace expansion

```
IntraLink/
  apps/intralink/           # current diagnostic CLI; eventual unified app entrypoint
  crates/intralink-core/    # current probe/role primitives; future identities, grants
  crates/intralink-transport/ # current direct probe; future authenticated session layer
  crates/intralink-vault/   # current: encrypted profiles and grants; planned: native OS keystore
  crates/intralink-discovery/ # planned extraction: discovery and DDNS currently in CLI modules
  crates/intralink-session/ # planned: authorization, revocation, audit, negotiation
  crates/intralink-platform/ # planned: traits and native adapters
  crates/intralink-media/   # planned: frame, audio, encoder/decoder, pacing
  crates/intralink-files/   # planned: explicit shares and resumable transfers
  crates/intralink-resource/ # planned: read-only health/metrics initially
  crates/intralink-terminal/ # planned: explicit authorized user-level sessions
  crates/intralink-editor/  # planned: text editing on explicitly shared paths
  crates/intralink-logwatch/ # planned: bounded monitoring of approved logs
  apps/ui/                 # planned: Tauri 2 shell, capability-driven UI
  platform/                # planned: Windows, Linux, macOS, Android, iOS adapters
  docs/                    # current: architectural decisions and release criteria
```

Planned entries deliberately have no empty crates or mock implementations. Add each module only when its behavior and tests are defined. Rust is the primary implementation language; Kotlin/Swift and operating system APIs are allowed when required by the platform. Tauri 2 is a proposed shell, not a verified dependency or a guarantee of support for legacy architectures. Headless Linux remains CLI-capable. Use capability negotiation to avoid offering unavailable operations.

## Media pipeline (preview and future)

v0.15 uses a demand-driven CPU path: locally selected display -> XCap snapshot -> aspect-preserving 540p/720p/1080p reusable RGB canvas -> exact change detection -> negotiated JPEG, exact Compact keyframe, basic or compact Rust tile updates, or repeat -> reliable QUIC with ordered receipt ACK and eight-record credit -> capacity-one changed-record receiver queue -> ordered decode into reusable presentation buffer -> MiniFB window. The network runs independently from the native UI/capture thread. No arbitrary encoded-frame dropping is allowed because tile updates depend on previous pixels. A fixed eight-record receipt window bounds conforming-peer unacknowledged media payload to 16 MiB, excluding application, QUIC and OS allocations. Static screens avoid duplicate compression/decoding; v0.13 reduces capture frequency after consecutive repeats and recycles consumed receive payload allocations. The workspace keeps one ordered decoder per active session and presents only the focused surface; priority feedback can reduce background capture to 5 FPS. See [WORKSPACE.md](WORKSPACE.md). This is not the final GPU video engine. See [TILES.md](TILES.md), [DESKTOP.md](DESKTOP.md) and [PERFORMANCE.md](PERFORMANCE.md).

Target pipeline: Capture -> GPU surface -> hardware encoder -> bounded frame queue -> QUIC media flow -> hardware decoder -> render. Avoid unnecessary CPU/GPU copies. Negotiate H.264, VP9 or AV1 based on actual hardware and distribution obligations. Use unreliable media datagrams only for data that tolerates loss, reliable streams for input/session control, files and required codec configuration. Detect congestion, adapt bitrate/resolution/frame rate and request keyframes. Opus for permitted audio. No codec claims until measured; licensing and patent review for distribution. A separate local FFmpeg codec feasibility tool and CPU measurements are documented in [CODECS.md](CODECS.md). A quality target is 1080p60 on supported modern hardware, not a guaranteed result. Report p50/p95 input-to-display latency, CPU/RAM, dropped frames, encode/decode latency, reconnect time and throughput.

## Local persistence (current and future)

See [VAULT.md](VAULT.md) for the current encrypted file format. SQLite is intentionally deferred.

SQLite tables: devices, device_addresses, local_grants, session_events, settings, migrations. Keep secret keys and provider tokens out of SQLite plaintext. Use platform keystore/Keychain/DPAPI/libsecret as available; encrypt confidential database fields or the entire vault. Protect offline exports with Argon2id and XChaCha20-Poly1305 using reviewed libraries, versioned parameters and authenticated metadata. No custom cryptographic protocol or secret export by default. SQLite is planned, not a dependency of the diagnostic.

## Local resource foundation (v0.11)

`intralink-core::resources` defines validated aggregate CPU and memory values independently of the OS. The optional `resource-monitor` feature in the CLI uses a single refreshed native `sysinfo::System`, only CPU and memory refreshes and finite foreground sampling. Missing values are explicit. No resource ALPN, remote command handler or persistent telemetry exists. A future remote read-only capability must have separate consent, authenticated access and revocation; this local command does not grant it. `intralink-core::canvas` handles validated reusable RGB output canvases without a redundant full clear.

## Shared direct routing policy (v0.12)

`intralink_core::network` is a standard-library-only routing policy shared by the CLI and QUIC transport. Numeric addresses bypass the resolver, mapped IPv4 uses IPv4 sockets, and normalized candidates are bounded and deduplicated. The owner-supplied WAN comparison is descriptive only. This change does not expand grants, enable relay or discovery defaults, or modify the vault/protocol. See [NETWORKING.md](NETWORKING.md).

In v0.16 the workspace presenter acquires an immutable snapshot of the focused surface and releases its lock before calling the native window. The ordered decoder can continue, using safe Rust copy-on-write only if a snapshot overlaps its update. F12 changes the local fullscreen monitor only; a window transition error does not tear down the independent sessions. See [WORKSPACE.md](WORKSPACE.md) for bounds, test scope and remaining native gates.

## v0.17 local overview

The workspace has one optional local 960x540 bitmap overview, composed from already decoded immutable surfaces. Opening it does not alter authorization or start additional streams. Pointer hit testing is restricted to the local overview; separate focus and slot activation actions feed the existing workspace policy. Periodic redraws are rate-limited and the allocation is reused; no thumbnails are persisted. The native GUI path remains a preview pending full integration tests. See [WORKSPACE.md](WORKSPACE.md).

## Current v0.22.0 desktop v8 update

The Windows optional Host capture backend now selects snapshot, persistent WGC or strict WGC GPU scaling only by local choice and fresh consent. The capture session starts on the first authenticated capture demand. The ordered media transport adds a distinct no-update record (no newly observed native generation) alongside changed and verified-repeat records, using v8 ALPN/request and unchanged codec IDs, bounded credit and vault/probe. Independent receiver surfaces remain unchanged on no-update; the local viewer adds a presentation-only last-frame notice. Native hardware encode/decode and physical multi-device validation remain pending.


## Restricted headless decode boundary (development, 2026-11-26)

The native Decoder owns its private CUDA context, parser and NVDEC surfaces, with an optional boxed SurfaceSink; the existing DXGI Presenter is only instantiated in the explicit preview path. The developer-only restricted_decoder_probe executable embeds an exact locally generated fixture, accepts a fixed nine-byte bootstrap and matches the trusted adapter LUID without constructing a D3D11 device or GUI. The same low-integrity launcher, job and mitigations are retained. Actual restricted Debug decoding of six 1080p access units passed. This is not the shipping remote worker or a shared-surface transport. Next validate parent-owned D3D11 resources exported as explicit NT handles, CUDA external resource import and bounded keyed-mutex/fence ownership on the same LUID; reject external handle values and never allow network-supplied resource descriptors. See NVIDIA CUDA Driver API external resource interoperability: https://docs.nvidia.com/cuda/cuda-driver-api/cuda_driver_api/group__CUDA__EXTRES__INTEROP.html . Independently qualify OS-enforced filesystem/network isolation with AppContainer (no network capabilities), based on https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer ; actual CUDA compatibility and permissions are not yet demonstrated.
