# Simultaneous screen and input sessions (desktop preview)

The desktop screen and input capabilities now have independent read-only session leases on the same encrypted profile. This is a two-process/two-window preview, not a combined viewer-control window or a new wire protocol. Windows and Linux X11 use their existing native adapters; Android retains the original exclusive vault and native session reservation.

## Operating the two capabilities

Pair the actual two machines using the existing signed invitation exchange. Review full identities on both machines. The saved outbound route points to the screen listener, for example private IPv4 port 45100. On the CLIENT choose the actual monitor ID from `intralink displays`. Open separate local interactive terminals on the CLIENT and independently approve each command, with the same existing vault:

    intralink share-desktop --bind CLIENT_PRIVATE_IPV4:45100 --controller base --monitor MONITOR_ID --seconds 300 --height 1080 --fps 60
    intralink input host --bind CLIENT_PRIVATE_IPV4:45101 --controller base --seconds 300 --advertise-lan

Windows input/2 may add `--display-geometry --entry-portal full` to the input host. Use separate available UDP ports; do not expose input to a public network. The actual private address and monitor ID replace placeholders. Each listener remains individually visible, locally approved, pinned to the approved BASE and bounded by its existing session deadline. Start the BASE within each listener's acceptance window.

On the BASE, open two local interactive terminals:

    intralink view-desktop --device client
    intralink input workspace --device client --input-port 45101

For Windows proportional KVM:

    intralink input workspace --choose --geometry --detect-displays --desktop-edges --raw-motion --compact-control --input-port 45101

The second command requires all selected CLIENTs to use the explicitly specified input port; use the existing capability-filtered discovery or saved route when not overriding. Only the input command changes its temporary route port. A DNS name is still only a route; pinning is unchanged. With an explicit input port, discovery hints with another port are discarded; private IPv4 restrictions remain. Screen settings/codec negotiation are independent. Use the same selected primary display for matching visual/input coordinates; do not assume arbitrary captured monitor and input target are the same. The existing full-screen foreground capture source covers the BASE while remote; the compact preview or focused pad can leave the view visible. No combined automatic pointer overlay on a video frame or transparent global capture yet.

Close the screen window/listener to end only screen sharing; close the input window/listener to end only input and release owned held input. Ending one capability does not revoke the other or grant it. Stop both before editing/revoking peers, changing roles or renaming the encrypted profile. No live grant mutation. Both session approvals use the existing directional pairing identity; persistent per-capability grant schema is not introduced. Never use this preview on sensitive, elevated or unattended desktops.

## Locking and security

- `Vault::lock` retains an exclusive OS file lock on the existing canonical-parent `.lock` sidecar; this remains mandatory for all mutations, diagnostics and older/Android exclusive commands.
- The desktop `SessionVault` is a separate typed read-only API. It first takes a shared lock on the existing profile lock, then the exclusive `.screen-session.lock` or `.input-session.lock` sidecar. It exposes authenticated load only, no save/initialize/role change or rename. Regular-file and symlink checks remain; the lock order is fixed and nonblocking. Failure to acquire a lane releases the partial shared lease. Lock files contain no identity, secrets, routes, grants, or session payload and are not deleted on close (do not delete lock files while sessions exist).
- One screen command (host, single view or workspace) and one input command (host, workspace or input layout/edge diagnostic) can coexist per profile, not two screen or two input commands. All input preference writes are still serialized by the exclusive input lane. Existing session code retains the guard through native/transport cleanup. All old exclusive commands fail closed while either lane is held. Different profile files and an attacker controlling the OS account are outside this local-lock guarantee.
- The native listener/stop windows, separate exact ALPNs, consent, protocol bounds, input activation and release ACKs are unchanged. Two endpoints can have the same pinned identities and distinct ports; no address sharing across these separate processes, implicit permission, relay, reconnect, or weakened protocol. Optional journal remains a separate exclusive writer for its existing commands.
- The tests use actual Windows file locks in independent subprocesses and actual same-identity QUIC endpoints. They independently stop input and screen, verify fresh generated frames or input records after the other closes, and verify mutation conflicts. No native capture/injection during these tests, no production or multi-device acceptance claim.


## Unified Windows BASE control (2026-11-28)

Current Windows input-preview adds `control-desktop --choose --input-port PORT` for one paired CLIENT, with a single foreground window/unlock and independent screen/input/2 start/stop/arm. See [UNIFIED-CONTROL.md](UNIFIED-CONTROL.md) for exact operating scope, primary monitor confirmation, coordinate mapping, local safety guards and test evidence. Separate CLIENT listeners, physical acceptance, authenticated video-source binding, unified multi-device control and Linux/Android UI remain open.
