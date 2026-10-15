# Infra v1 implementation status

This file records what exists in code. Product intent remains in the other design documents.

## 2026-10-15 — native control + product UX foundation

### Implemented

- Native Rust workspace with `protocol`, `gateway`, `agent`, `platform`, `local-state`, and `cli` crates.
- MCP Streamable HTTP gateway with compact `devices`, `exec`, `files`, `search`, `processes`, `jobs`, `terminal`, and `transfer` tools.
- WebSocket/WSS outbound agent connection with reconnect/backoff and connection-generation supersession.
- SQLite gateway persistence for enrolled devices, last-seen/revocation state, one-time enrollments, reconciliation snapshots, and bounded audit metadata.
- Offline enrolled devices remain visible after gateway restart.
- One-time enrollment codes are random, short-lived, stored only as SHA-256 hashes, single-use, and idempotent for the same enrollment request/public key.
- Per-device Ed25519 identity generated locally by the agent and persisted separately from the gateway.
- Fresh challenge-response authentication on every signed device connection; the private device key is never transmitted.
- Independent device revocation at the gateway. Legacy shared-token authentication remains only as an explicit development/migration fallback.
- Bounded user-level one-shot `exec` with explicit `outcome_unknown` rather than unsafe automatic replay.
- Text file operations: byte-range read, atomic-style write, append, exact edit with replacement-count safety, SHA-256 hashing, list, stat, mkdir, copy, move, and explicit permanent delete. Binary/invalid-UTF-8 text reads fail clearly. `trash` currently reports unsupported.
- Native filename/content search with regex or literal matching, git-ignore awareness, hidden-file control, glob filtering, context lines, bounded results, and binary/large-file skipping.
- Cross-platform process listing/filtering and direct process-tree termination without shell interpolation.
- Durable agent-side jobs with separate worker processes, metadata/output spooling, byte-offset reads, bounded retained output, cancellation, boot identity, and reconnect reconciliation.
- Real PTY/ConPTY sessions using `portable-pty`: open/list/read/write/resize/close, byte-offset transcript reads, bounded rolling transcript, independent worker process, loopback capability-token control, boot identity, and reconnect reconciliation.
- Resumable binary transfer with persistent metadata: upload begin/chunk/commit, download begin/chunk, status/list/cancel, offset resume, idempotent upload chunk retries, declared-size enforcement, SHA-256 verification, staging files, and final rename.
- Active job/terminal/transfer reconciliation sent after reconnect and persisted by the gateway.
- Local `config.json` and `runtime.json` state shared by agent and CLI. Runtime status records PID, connection state, stable device identity, generation, last successful connection, and sanitized last error.
- `infra` CLI: `status`, `doctor`, `install`, `repair`, `reconnect`, and `uninstall`, including JSON status/doctor output.
- Startup adapters implemented for Linux systemd user services, macOS LaunchAgents, and Windows logon Scheduled Tasks.
- Native bootstrap installers: `scripts/install.sh` and `scripts/install.ps1`. Development/source mode is testable without releases; release mode verifies SHA-256 sidecars before installing binaries.
- GitHub tag-release workflow defined for Linux x64/ARM64, macOS Intel/ARM64, and Windows x64/ARM64 assets plus SHA-256 sidecars.
- Embedded minimal dashboard served by the gateway with Overview, Devices, Sessions, Activity and Settings, Add Device enrollment flow, generated OS install commands, responsive layout, SSE refresh, and polling fallback.
- Dashboard APIs expose persisted devices, reconciled active work and bounded audit metadata. Audit records action/outcome/request IDs only, not stdout, file contents, enrollment secrets, or private keys.
- Static dashboard/bootstrap responses use no-store, nosniff and a restrictive Content Security Policy.
- Structured JSON tracing and Linux/macOS/Windows Rust CI definition.
- Isolated `scripts/smoke-v1.sh` verifies enrollment, signed authentication, MCP `exec`, identity persistence, and reconnect.
- Isolated `scripts/smoke-install-dashboard.sh` verifies dashboard assets, enrollment, the native installer flow, persisted config/identity, online reconnect, status, doctor, sessions API and activity API.

### Current verification

On the Linux ARM64 development host:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace` — 34 Rust tests, including real PTY/job workers, search/edit/hash and resumable transfer tests.
- `./scripts/smoke-v1.sh`
- `./scripts/smoke-install-dashboard.sh`

The previous cross-platform Rust workflow passed Ubuntu and macOS. Windows compiled and passed clippy but its PTY test used the Unix `cat` command; the test now selects `more` on Windows and must be confirmed by the next CI run.

### Intentionally incomplete

- Production MCP/dashboard OAuth and a proper browser login/session. The gateway still refuses non-loopback binding by default unless an explicit development override is supplied.
- OS-native Keychain/DPAPI/Secret Service storage and Windows ACL hardening. The current state directory/file permission hardening is strongest on Unix.
- Cryptographically signed release/update metadata. Bootstrap release installs currently verify SHA-256 assets; the steady-state updater and rollback chain remain to be built.
- `infra update`, staged binary replacement, health-check rollback, sanitized log export, and permission-specific doctor checks.
- Real-device installer/startup testing across macOS and Windows. Startup adapters compile cross-platform, but the isolated Linux smoke intentionally skips service registration.
- Unified patch, line-range read, tail/watch and recoverable platform trash.
- Higher-level device-to-device transfer orchestration and throughput reporting. Binary device upload/download/resume exists at the MCP layer.
- Structured system/services/network/container/Git modules.
- Privileged helper and semantic desktop automation.
- Full dashboard device-detail/terminal/files/permission/diagnostic controls. The current dashboard is a functional control/status foundation, not the finished product.
- Guaranteed PTY/job survival through every service-manager restart. Workers survive MCP/gateway/network loss and ordinary controller exit, but installer/service-manager behavior still needs real-device validation.
- PTYs/jobs never claim to survive a full machine reboot; stale-boot entries are marked interrupted.
- Windows atomic replacement semantics need a platform-specific upgrade; overwrite may require destination removal before rename.

## Migration safety

All v1 development remains side-by-side. The deployed Node v0.3 remote-control service has not been modified, restarted, or migrated by this branch.
