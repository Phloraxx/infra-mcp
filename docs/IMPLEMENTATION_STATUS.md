# Infra v1 implementation status

This file records what exists in code. Product intent remains in the other design documents.

## 2026-10-15 — foundation + persistent-control slice

### Implemented

- Native Rust workspace with `protocol`, `gateway`, `agent`, and `platform` crates.
- MCP Streamable HTTP gateway with compact `devices`, `exec`, `files`, `processes`, `jobs`, and `terminal` tools.
- WebSocket/WSS outbound agent connection with reconnect/backoff and connection-generation supersession.
- SQLite gateway persistence for enrolled devices, last-seen/revocation state, one-time enrollments, and latest reconciliation snapshots.
- Offline enrolled devices remain visible after gateway restart.
- One-time enrollment codes are random, short-lived, stored only as SHA-256 hashes, single-use, and idempotent for the same enrollment request/public key.
- Per-device Ed25519 identity generated locally by the agent and persisted separately from the gateway.
- Fresh challenge-response authentication on each signed device connection; the reusable private key is never transmitted.
- Independent device revocation at the gateway.
- Legacy shared-token authentication retained only as an explicit development/migration fallback.
- Bounded user-level one-shot `exec` with explicit `outcome_unknown` rather than unsafe replay after an ambiguous timeout.
- Text file operations: byte-range read, atomic-style write, append, list, stat, mkdir, copy, move, and explicit permanent delete. Binary/invalid-UTF-8 text reads fail clearly. `trash` currently reports unsupported.
- Cross-platform process listing/filtering and direct process-tree termination without shell interpolation.
- Durable agent-side jobs with separate worker processes, metadata/output spooling, byte-offset reads, bounded retained output, cancellation, boot identity, and reconnect reconciliation.
- Real PTY/ConPTY sessions using `portable-pty`: open/list/read/write/resize/close, byte-offset transcript reads, bounded rolling transcript, independent worker process, loopback-only capability-token control channel, boot identity, and reconnect reconciliation.
- Active job/terminal reconciliation sent by the agent after reconnect and persisted by the gateway.
- Structured JSON tracing and Linux/macOS/Windows Rust CI definition.
- Isolated `scripts/smoke-v1.sh` verifies enrollment, signed authentication, MCP `exec`, identity persistence, and reconnect without touching the live v0.3 deployment.

### Verified on the Linux ARM64 development host

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- real PTY open/write/read/resize/close test
- real job worker spool/completion test
- SQLite persistence/reopen tests
- enrollment expiry/single-use/idempotency tests
- signed-auth/revocation tests
- connection supersession tests
- isolated gateway + real agent enrollment/reconnect + MCP exec smoke test

### Intentionally incomplete

- Native OS credential stores/DPAPI/Keychain/Secret Service. The first implementation uses a private state directory and a `0600` identity file on Unix; Windows-specific ACL hardening is still required.
- Production MCP/dashboard OAuth. The gateway refuses a non-loopback bind by default unless an explicit development override is set.
- Installer/startup integration, `infra doctor`, updater/rollback, and dashboard.
- Search, exact edit/patch, tail/watch, binary/resumable transfers, hashes, and device-to-device transfer.
- Recoverable platform trash.
- Structured system/services/network/container/Git modules.
- Privileged helper and desktop automation.
- Persistent global session-history tables in SQLite; the gateway currently stores the latest device reconciliation snapshot while authoritative execution/session state remains on the device.
- Guaranteed job/PTY survival through an OS service-manager restart. Worker processes survive MCP/gateway/network disconnect and an ordinary controller process exit, but a future installer/supervisor must ensure service cgroups/job objects do not terminate children during an intentional agent service restart.
- PTYs/jobs never claim to survive a full machine reboot; stale-boot entries are marked interrupted.
- Windows atomic replacement semantics need a platform-specific upgrade; the current fallback may remove an existing destination before rename.

## Migration safety

The v1 work remains side-by-side. The deployed Node v0.3 remote-control service has not been modified, restarted, or migrated by this branch.
