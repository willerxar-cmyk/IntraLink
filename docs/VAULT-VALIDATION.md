# Vault validation — v0.20.0

## Implemented change

Normal saves authenticate the existing vault before staging replacement, then compare public installation identity and role. This deliberately adds an Argon2id verification to a local write operation rather than trusting that every caller has already unlocked the original. The format and cryptographic dependencies are unchanged. Failed authentication must leave the old file unchanged; explicit identity rotation and password changes need a separate workflow. The local trust and rollback limitations in [VAULT.md](VAULT.md) still apply.

## Executed on Windows

The exact vault source archive was transferred and checked against SHA-256. All five vault Rust source hashes were independently checked against this repository. The real core and vault crates were compiled with Rust 1.96.0 in an isolated workspace. No personal vaults, personal keys or live desktop capture were used. Dependencies were resolved from Cargo; this validation workspace is not a complete application lockfile.

| Gate | Result |
|---|---|
| `cargo test -p intralink-vault` | 11 passed, 0 failed (20.59 s test time) |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Exit 0 |
| `cargo test --locked --workspace` | 65 core + 11 vault tests passed, 0 failed, exit 0 |

The two new storage regressions check rejection of wrong passwords, different identities and tampered ciphertext with byte-for-byte preservation. The existing tests exercise encryption/decryption, invitation signing and expiry, local roles, legacy fields, locking, identity persistence, revocation and replay state. The role-preservation condition is implemented; no separate same-key role-change regression was executed. Detailed commands, timing and source hashes: [vault-windows-validation.json](../benchmarks/vault-windows-validation.json).

## Remaining release gates

Transfer and build the actual transport and native app, generate and review the complete lockfile, run the full workspace formatting, Clippy, tests and release builds. Validate authorized QUIC loopback, identity rejection, enrollment-to-transport integration, consent and cancellation, multi-host streaming, native input and GPU acceleration before claiming production readiness. This is not an independent security audit. The extra KDF verification is not in the media path; its local UX cost should be measured separately.
