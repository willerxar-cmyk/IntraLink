# Local session-attempt history (development preview)

The journal is **opt-in per foreground command**, disabled by default. It records command attempts, not independently authenticated access events. It is an initial operational-history feature, not a production security audit trail. The existing local approval, pinned identities, role restrictions and direct-only transport are unchanged.

## Usage

Use an initialized test profile in a private directory under your local OS account. The usual owner approvals are still required. Examples, after the corresponding build and native test gates pass:

```sh
intralink --vault controller.vault --record-session connect --device test-host
intralink --vault controller.vault history
```

With the optional desktop preview enabled and a previously approved test device:

```sh
intralink --vault controller.vault --record-session view-desktop --device test-host
```

`listen` and `share-desktop` also accept `--record-session`. Existing examples in [DESKTOP.md](DESKTOP.md) show the required options. Other commands refuse that flag. A history command never opens the vault or a network connection. It can be used to inspect recorded metadata even if the vault is locked or has been removed. A history file does not identify the vault's cryptographic identity; replacing a profile at the same path can mix its history with prior attempts. Use a separate profile path and archive the journal when replacing a profile.

## Data and behavior

The sibling `<vault filename>.history` stores ASCII lines with a version marker (`IL1`), local monotonically increasing attempt number, start UTC Unix seconds, fixed operation category and state. The completion record also includes monotonic elapsed milliseconds. Wall-clock timestamps can be inaccurate or go backwards. The file does not store a device alias, endpoint ID, DNS name, IP address, password, token, key, command text, error details, screen pixels, input or any other session content. There is no external destination.

- `begin`: a local command attempt started recording, before vault unlock, local approval and network activity. It does not prove access.
- `protocol-completed`: the command returned a completed diagnostic exchange or a desktop transport completion; it does not establish how many frames were presented, that a user observed them, or that a security audit passed.
- `local-cancel`: a desktop foreground cancellation returned normally.
- `command-error`: an operation returned an error, including possible local approval refusal, configuration errors or network errors.
- A `begin` without a completion record is displayed as **unknown outcome**. Killing the process or failing to persist completion must not be misreported as a successful session or cancellation.

The journal is bounded to 256 KiB. It reserves space for both records before starting the requested operation. It does not rotate or silently overwrite history. If it is full, malformed or busy, the requested recorded command fails before its future is polled. The unrecorded command remains available by omitting the opt-in flag. Archive or remove a journal manually only when no command is using it. If completion persistence fails, the command reports a local history error, but this cannot undo an operation that already happened. A partial record is treated as corruption, not silently repaired.

## Storage and threat model

The journal uses an exclusive advisory file lock for a recorded command and a shared lock when reading. Records are appended and synchronized to disk; the Unix parent directory is also synchronized after the begin record. The parser is bounded and rejects unsupported categories, arbitrary text, malformed records and invalid sequences before display or append. Existing Unix files must be private and have a single hard link. New Unix files request mode 0600. Existing symbolic links are refused. This is not protection against an adversarial process with write access to the parent directory; filesystem path races, Windows reparse points/ACLs, abrupt power loss and filesystem semantics require native tests and trusted local storage.

The history is **plaintext local metadata**, not encrypted, authenticated or tamper-evident. An OS user with sufficient access can read, edit, delete, replace or roll it back. A compromised computer can falsify its own history. It must not be used as a compliance log, authentication source or permission grant. A `--record-session` invocation can create history before a profile is unlocked, including if the requested profile does not yet exist. No native keystore, signed event chain, retention automation, authenticated peer linkage or live revocation is implemented in this milestone.

## Validation

The actual parser is included in the dependency-free Rust harness. The app tests use temporary files, not user data. On real Windows/Linux/macOS run the workspace tests and verify native lock behavior, privacy of the parent directory, cancellation, process death, disk-full/corruption and opt-out. Desktop preview tests are limited to its currently supported operating systems. No Rust compilation or native execution was available in the authoring environment.
