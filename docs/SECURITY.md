# Threat model and security gates

Only operate on devices you own or administer with authorization. A user must be able to see, revoke and terminate access. Do not conceal an agent, evade OS consent, circumvent security controls or impersonate a different application.

## Threats and controls

| Threat | Required production control |
|---|---|
| DNS poisoning / changed IP | Independently pinned peer public key; TLS identity verification, no trust-on-first-use |
| Lost controller | Local host-side revocation and termination of sessions; per-controller identity, short-lived grants where appropriate |
| Compromised host | No shared private credentials, controller inventory, central provider token or automatic host-to-controller privilege |
| Pairing interception | Local confirmation, single-use expiring invitation, independent fingerprint comparison, channel binding |
| Replay | Fresh session authentication, expiry, unique session IDs; no early-data privileged commands |
| Unauthorized files or shell | Deny by default; explicit capability and local policy checks; constrain file roots, symlink handling and privileges |
| Resource exhaustion | Handshake/rate limits, frame lengths, bounded queues, concurrent session limits, cancelation and watchdogs |
| Malicious updates | Signed release metadata, verified signatures, anti-rollback policy, dependency review and reproducible builds where practical |
| Unobserved access | Persistent visible indicator during sessions, local disconnect, auditable permission changes and access events |
| Excessive data collection | No session payload logging by default, no telemetry by default, documented retention and local control |

## Implemented in the preview

- Persistent random identity per local encrypted profile; peer public identity checked by each local operator; see [VAULT.md](VAULT.md).
- Targeted signed offline invitation and acceptance with separate signature domains, 60–900 second lifetime, 128-bit nonce, size bounds, explicit confirmation, and encrypted replay state. No network enrollment API. Clock rollback and old vault restoration remain risks; a canceled invitation cannot grant host access, but the controller may retain an unused outbound grant until revoked.
- Optional IPv4 DNS-SD advertises public host ID/address/port; visible to other LAN participants. Spoofed advertisements never authorize access, but may cause delay or misdirect connection attempts. Neither private keys nor device inventory are advertised.
- Iroh authenticated QUIC transport and pinned host ID; host rejects an unapproved controller ID before application data.
- Fixed messages with small byte limits and handshake/session deadlines; one-shot listener.
- No system privileges, background service, file or shell operations, auto-updater or silent access. Saved grants alone authorize diagnostic probes only. The new view listener additionally requires per-session local consent. Revocation is local and applies to subsequent sessions; stop an active listener first.
- No default relay or directory; no global provider token stored or distributed by this code.
- DDNS HTTPS request accepts no redirects or system proxy; errors do not print token-bearing request URLs. The hidden token is wrapped in a zeroizing buffer, but HTTP libraries and the provider necessarily handle copies. This is not a guarantee of complete in-memory secret erasure. System proxies, debugging tools and provider logs may have access to relevant metadata; use trusted local systems.

## Experimental screen sharing (v0.9)

- Separate ALPN, pinned controller and host identities, exact request including approved codec mode and one selected local display. No capture before peer validation, and no implicit view grant or automatic startup.
- Foreground host prompt names the peer and warns about private display content. A visible, topmost sharing window is opened before capture; a native window failure aborts. No session payloads are intentionally written to disk by IntraLink. A recipient may independently record the display. Do not share sensitive content.
- Frame length, count, time and decoding dimensions are bounded. Configuration is checked before capture or decode; repeat before first frame and invalid ACKs fail closed at receipt validation. An eight-record credit window permits at most eight records before the first consumed ACK, so failure is not necessarily immediate. The v7 priority bit only reduces sampling, not authority. Receipt ACKs are not identity or consent and cannot grant new capabilities. No generic remote command dispatcher, input injection or privileged capture bypass. Linux Wayland is refused by the preview rather than bypassing the compositor.
- Local Escape/close and Ctrl+C stop are provided. Network waits run in a separate task; the foreground window can cancel that task without waiting for a new network record. Native capture, encoding, decoding and OS calls can still block their owning thread; native UI responsiveness is not certified. Topmost behavior, lock-screen behavior, cleanup, malformed JPEG handling and disconnect timing require real native tests. The vault is locked for the session; stop before revoking. No live revocation or permanent desktop grants yet.
- Tile updates are allowed only after a keyframe in hybrid mode; validated indices, palette indices and padding, run coverage, exact lengths and full pre-validation prevent malformed tile records from mutating the presentation buffer. The ordered queue avoids discarding required encoded updates. A Compact lossless keyframe must cover the entire grid, with strictly increasing indices; it is validated before resizing or modifying the canvas. The codec is uncompiled and needs fuzzing.
- A malicious paired peer still supplies untrusted compressed bytes; the decoder limits are not a sandbox or security audit. A hostile connection can occupy the single acceptance window. Restrict the preview to isolated owner-controlled test networks.

## Required before any production release

Review the Iroh and TLS dependency configuration, validate network egress by packet capture, test DNS spoofing and mismatched keys, replace passphrase-only key handling with native keystore support, add live revocation and rollback defenses, verify platform privacy/permission UX, test invitation parsers under fuzzing and malicious peers; review mDNS dependency and interface scope, test expiry, clock changes, rollback and repeated grants; audit dependency tree and licenses, sign binaries, and run an independent security review. The preview has not been compiled, tested or security-audited in its authoring environment.

## Routing validation (v0.12)

Normalize mapped IPv4 before applying destination filters or binding socket families. Reject unspecified, multicast, limited broadcast, IPv4 0/8, zero ports and unsupported interface-scoped IPv6. Candidate selection remains bounded. Route validation never authorizes a peer; both QUIC identities still need to match local grants. A valid-looking address is not proof of ownership, and a same-family WAN/DNS match is not proof of reachability. No public-IP service, NAT traversal, automatic port forwarding or remote privilege has been added. Native tests remain pending.

## Optional local attempt history (v0.14)

The foreground operator may record local command attempts with `--record-session`. The journal stores only operation category, local time, sequence number, outcome and elapsed time. It is separate from the encrypted vault and has no peer identity, alias, address, secret or payload field. It is not telemetry. This plaintext journal requires a trusted, private local directory; it is not encrypted, signed, tamper-proof or a proof of access. The native permission and crash-recovery tests remain open. No privileges are gained by enabling or editing it. See [SESSION-HISTORY.md](SESSION-HISTORY.md).

## v0.20 storage regression tests

Normal vault updates now authenticate the old envelope and preserve public identity and local role before replacement. Two new storage tests verify that wrong passwords, different identities and corrupted old ciphertext leave the existing bytes intact. Eleven real vault tests passed on Windows; this is regression coverage, not a cryptographic audit. No unattended access, network privileges, or new wire operations were introduced. See [VAULT-VALIDATION.md](VAULT-VALIDATION.md).

## Explicit WGC Host and no-update semantics (desktop wire v8)

Host backend choice is a local CLI option shown before fresh approval. Persistent capture starts only after the existing authenticated peer and exact approved codec request and after the red local stop window is painted. The source is selected by enumerated native monitor identity, not an arbitrary handle or controller-supplied display ID. Identity/geometry mismatch, callback/driver failure, closure and prolonged native silence stop the preview without capture-backend fallback. The native indicator defaults are unchanged. No-update records are ordered and bounded, may occur only after the first media frame, and are neither verified repeats nor evidence of capture health. The 15-second no-observed-readback limit is conservative; it may terminate truly idle sources. The viewer distinguishes the last frame from a new native sample. No new privileged remote command, input, file, shell, background listener or vault format. Native lock/suspend, hotplug, driver-loss and real full-display authorization acceptance still require testing.

## Isolated H.264 v9 diagnostic

The separate H.264 ALPN is restricted to loopback bind/candidate addresses, exact local profile agreement and pinned identities. Normal v8 Host/viewer never select it. Both sender and receiver reject malformed bounded access units, gaps and inconsistent parameter sets before forwarding to the consumer or sending credit. This is an envelope/partial header check, not a complete AVC validation or a security boundary for the native decoder. Only byte-identical locally generated fixtures enter NVDEC. No remote input/control, background access, new capability grants or external codec listener is exposed. Before any physical network enablement, qualify full bounded SPS/slice semantics, untrusted bitstream isolation, lifecycle and teardown on real devices, and perform a security review. Do not infer that loopback is itself a trust boundary.

### Continuous native diagnostic containment

The v9 SPS/PPS parser rejects incompatible levels, geometry, color, timing, reference counts and bounded syntax before accepting a changed record. Full CABAC/slice/SEI validation and native decoder hostile-input containment remain open. The live diagnostic uses two ephemeral pinned loopback endpoints and an independent bounded in-process native-origin token channel. Each received access unit is compared byte-for-byte with its exact original, including sequence, profile and keyframe flag, before submitting the locally produced token to CUVID. No arbitrary network AVC, physical display, vault, remotely accessible listener or new privilege. Normal share/view remain v8-only. No local stop can force-interrupt a stalled synchronous driver call; process-level verification has a deadline, and production codec isolation and driver-error teardown remain mandatory. Cancel aborts the decoder without EOS to avoid publishing delayed pictures. All native workers join on ordinary cancellation; fatal native failures are not concealed.

### Isolated native decoder process (diagnostic)

The explicit --isolated-decoder mode of desktop-wgc-live-check uses a separate process, not an extra privileged service. The parent still authenticates pinned loopback peers and compares every compressed access unit against its exact native-origin token before handing that token to the child IPC sender. The hidden child entry accepts no vault/session-journal arguments and creates no network endpoints. It waits for a bounded versioned bootstrap, checks its Windows Job Object policy before initializing native devices, and independently validates every bounded v9 envelope/SPS/PPS/sequence before submitting it. No wire-controlled path, command, executable, adapter configuration or external address is accepted. The supervisor starts only the current executable, with inherited private stdin/stdout pipes, and an unnamed non-inheritable job (active process limit 1, kill on last job handle close). Native ACK requires an immediate matching decode/display callback; completion requires exact stage counts and actual zero process exit. One IPC request at a time, explicit bounded startup/frame/exit waits, and termination of the owned child on protocol failure or timeout. A normal cancellation sends an explicit stop, does not flush delayed pictures, and joins the pipe worker. This provides process lifetime/crash containment, **not a restricted-token/AppContainer sandbox**. The child still runs under the same OS user and driver/kernel; a local caller who deliberately constructs the same job and private protocol is not an adversarial security boundary. Environment/token and driver isolation, full hostile AVC qualification, and physical Host/Viewer acceptance remain mandatory before arbitrary remote AVC is enabled. No production sharing mode or permission is added.

### Real child-loss and recovery check

The optional developer tool `tools/native_decoder_fault_verify.py` (requires psutil in the test environment, no application dependency) launches only the generated-window diagnostic, reads its announced decoder PID, verifies the exact executable, direct parent identity and private worker command, then terminates that specific owned child. It requires nonzero parent exit, no false completion, capture/device/apartment release and actual decoder exit. Debug fault run: parent 36440, child 29724, parent exit 1, source delivered 1 and encoded 2, IPC error 232. A new independent 1080p 60-frame session then completed 60/60 Present submissions, child exit 0 and parent exit 0. Evidence: benchmarks/decoder-process-fault-debug-20261126.json and decoder-process-recovery-debug-20261126.txt. No GPU reset or other process termination. This is actual abrupt process-loss handling, not a hardware GPU hang or hostile-bitstream test.
