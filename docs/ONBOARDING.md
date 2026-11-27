# Local device onboarding

A BASE controls other devices (internal Controller role). A CLIENT receives explicitly approved access (internal Host role). Existing advanced Hybrid profiles remain supported by CLI. Role selection is not permission to connect or capture.

In the next Windows/Linux desktop candidate, run the executable with no arguments from an interactive terminal and no existing default vault, or run the setup command. The graphical chooser has two cards, keyboard shortcuts 1/2 and Escape cancellation. It only reads input within the focused local window. After selecting, the local terminal requests a device name and a non-echoed passphrase twice, then explicit APPROVE. CLI-only builds provide the same two choices in the terminal. No automatic device grant, discovery, capture, or listener starts. A noninteractive invocation cannot create an identity. Existing profiles are never overwritten by setup.

Commands (Windows uses intralink.exe):

    intralink setup
    intralink setup --role base --name Workstation
    intralink identity
    intralink set-role --role client
    intralink rename-device --name Studio

The role aliases base/client also work with init and set-role. Advanced CLI roles are controller, host and hybrid. The current graphical chooser is a first-run role selector, not a complete GUI dashboard. The name and passphrase stage remains terminal-based on desktop. Pair and start each capability using the existing explicitly approved commands.

## Authenticated role migration

The local passphrase and exclusive vault lock are required. Review previous/new role, count of grants to revoke and whether an outstanding invitation will be canceled; type APPROVE to proceed. Compatible grants, durable public identity, name and invitation replay receipts survive. Incompatible grants are revoked, never silently converted or restored. A changed role invalidates any outstanding invitation. The ordinary save API still rejects identity or role changes. No-op migrations leave ciphertext unchanged.

The optional UTF-8 device_name field is stored inside the existing authenticated encrypted envelope. Legacy unnamed schema-1 profiles load normally. Historical controller invitation receipts remain inert in a host-only profile, preventing replay if it later becomes a controller. Older readers may reject named profiles or host-only historical receipts. Do not downgrade a migrated vault; keep a private encrypted backup and review grants when restoring. No new wire format or automatic remote authority.

## Qualification

The vault tests verify wrong password, stale impact, compatible and revoked grants, pending invite invalidation, replay after role roundtrip, identity, name persistence, no-op and ordinary save rejection. CLI smoke tests reject noninteractive setup without creating a vault. The desktop render test checks role card hit geometry and distinct selection colors. Native interactive use and accessibility of the chooser are not yet physically qualified.
