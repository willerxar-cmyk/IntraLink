# IntraVault v2 envelope, profile schema 1

This is a development format, not a security-audited production keystore. In v0.3, the user supplies a strong passphrase for each local command. No account or service is required. Native secure storage (Keychain, DPAPI, Android Keystore, Secret Service) and recovery UX remain future work.

## Format

| Bytes | Description |
|---|---|
| 0–7 | ASCII `ILVAULT2` |
| 8–23 | 16-byte random Argon2id salt |
| 24–47 | 24-byte random XChaCha20 nonce |
| 48–end | Ciphertext and 16-byte Poly1305 tag |

Argon2id version 0x13, 64 MiB, three iterations, one lane and 32-byte output. No KDF costs are read from disk. The entire 48-byte header is authenticated as AAD. Salt and nonce are generated from OS randomness on every write. File maximum 1 MiB. Decrypted JSON schema 1 contains local role, 32-byte private key, and at most 128 directional grants. The key and metadata are not intentionally persisted in plaintext. Key/passphrase/decrypted buffers are zeroized where explicitly managed, but the serializer, OS, dependency internals and process copies mean this is not a complete memory-erasure guarantee.

## Grants

Inbound grants contain only the controller's public ID and local label; no route or inventory. Outbound grants contain a pinned host ID and a DNS/IP plus port. Host mode rejects outbound grants, controller mode rejects inbound grants. Hybrid grants are direction-separated within one local vault; a compromise of that hybrid profile can compromise both roles. Every grant authorizes only a diagnostic probe. The owner must separately approve each side, and explicitly start the foreground host listener. There is no remote enrollment endpoint.

## Enrollment state and backward compatibility

Schema 1 now has additive `pending_invite` (at most one signed-offer body) and `used_invites` (at most 128 accepted nonces with expiry). Both default to empty when absent in v0.2. The host consumes the matching pending body in the same encrypted save as its inbound grant. The controller records the nonce in the same save as its outbound grant, and prunes expired receipts when importing another offer. The v0.2 reader is strict and rejects newly saved fields. Do not downgrade or restore earlier vaults without reviewing all grants and invalidating outstanding codes. See [ENROLLMENT.md](ENROLLMENT.md). The encryption envelope and key identity do not change.

## Storage and recovery

Store in a private, local, user-owned application data directory, not on a shared or untrusted filesystem. The sibling `.lock` file is held with an advisory exclusive OS lock across local operations, including a single diagnostic session. Do not delete the lock file while a process is using it. Same-directory temporary ciphertext is synced, then persisted without overwriting on initialization or replaced on updates. Unix directory sync is requested after the rename; Windows crash recovery and directory ACLs still need native tests. Path checks reject observed symbolic links but do not defend against a concurrent local administrator or an attacker with write access to the parent directory. The threat model assumes a trusted OS account and private directory.

Back up the encrypted file offline when the app is closed; never share the passphrase. Loss of both key material and backup requires a new identity and fresh pairing. Old backups can restore revoked grants; there is no monotonic rollback counter. Review all restored grants. Revocation applies to future sessions; close a foreground listener with Ctrl+C before revoking. No live revocation daemon or audit log yet.

For production, add native keystore-backed key protection, audited invite lifecycle, per-session scoped capabilities, secure recovery, rollback protections and independent review before privileged operations are implemented.

## Authenticated updates in v0.20

A normal `save` first authenticates and validates the existing ciphertext with the supplied passphrase. It then checks that the new profile has the same public identity and role. Only after these checks can the existing atomic staging/replacement path run. This prevents the ordinary API from overwriting a vault with an incorrect passphrase, a different identity, or after authentication failure. It does not protect against a local attacker who can directly replace/delete files, rollback, or non-cooperating processes racing the filesystem. The extra KDF invocation increases local save cost and does not affect the video or network hot path. A separate authenticated password change, key rotation and backup/restore workflow remains to be designed. See [VAULT-VALIDATION.md](VAULT-VALIDATION.md).

## Named identities and explicit role migration

The next candidate accepts the optional device_name (1-64 UTF-8 bytes, no control characters) in encrypted schema 1. Missing fields remain valid. The separate authenticated Vault.change_role operation reviews the expected impact under the same exclusive lock, retains the identity, preserves compatible grants and historical replay receipts, revokes incompatible grants and clears pending invitations on a real change. Historical receipts in a client-only profile are inert until a later controller role. A no-op changes no ciphertext; a normal save still rejects role or identity changes. Old strict readers may reject named profiles and host-only historical receipts; do not downgrade. See ONBOARDING.md.
