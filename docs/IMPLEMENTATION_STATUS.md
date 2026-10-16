# Infra v1 implementation status

This file records what exists in code. Product intent remains in the other design documents.

## 2026-10-15 — native control + product UX foundation

### Implemented

- Native Rust workspace with `protocol`, `gateway`, `agent`, `platform`, `local-state`, and `cli` crates.
- MCP Streamable HTTP gateway with compact `devices`, `exec`, `files`, `search`, `processes`, `jobs`, `terminal`, `transfer`, `system`, `network`, `services`, `containers`, and `git` tools.
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
- Real PTY/ConPTY sessions using `portable-pty`: open/list/read/write/resize/close, byte-offset transcript reads, bounded rolling transcript, independent worker process, loopback capability-token control, boot identity, and reconnect reconciliation. Windows ConPTY startup terminal queries (DSR/DA1) are answered by the worker path rather than exposed as transcript noise. macOS durable PTY/job workers are submitted as independent launchd jobs so controller kickstarts do not tear them down; non-Linux boot identity uses the OS boot timestamp rather than controller PID.
- Resumable binary transfer with persistent metadata: upload begin/chunk/commit, download begin/chunk, status/list/cancel, offset resume, idempotent upload chunk retries, declared-size enforcement, SHA-256 verification, staging files, and final rename.
- Active job/terminal/transfer reconciliation sent after reconnect and persisted by the gateway.
- Structured system operations via `sysinfo`: machine/CPU/memory/load summary, disks/filesystems, and network-interface counters/addresses.
- Structured network operations: DNS resolution, bounded TCP reachability checks, and listener enumeration via direct platform utilities (`ss`, `lsof`, or `netstat`) without shell interpolation.
- Structured service operations: list/status/start/stop/restart using systemd-user on Linux, current-user launchd domain on macOS, and Windows SCM as the current OS user. These do not bypass OS privilege checks; broader elevated service control remains a privileged-helper task.
- Structured Docker operations: list, inspect, bounded logs, start, stop and restart through direct argv calls. Inspect output redacts sensitive environment/key fields by default.
- Structured Git operations: status, diff, log, branches, remotes, fetch, fast-forward-only pull, and push through direct argv calls with interactive credential prompts disabled. HTTP remote userinfo is redacted from returned URLs/output.
- Local `config.json` and `runtime.json` state shared by agent and CLI. Runtime status records PID, connection state, stable device identity, generation, last successful connection, and sanitized last error.
- `infra` CLI: `status`, `doctor`, `install`, `repair`, `reconnect`, and `uninstall`, including JSON status/doctor output.
- Startup adapters implemented for Linux systemd user services, macOS LaunchAgents, and Windows logon Scheduled Tasks. Linux startup reconstructs the standard user-bus environment for non-login/remote shells, and idempotent reinstall/repair reloads and restarts the controller so a replaced binary actually becomes active. Linux uses `KillMode=process` so durable PTY/job workers outlive controller-only restarts; `infra uninstall` explicitly terminates persisted Infra worker trees to avoid leaking them.
- Native bootstrap installers: `scripts/install.sh` and `scripts/install.ps1`. Development/source mode is testable without releases; release mode verifies SHA-256 sidecars before installing binaries.
- GitHub tag-release workflow defined for Linux x64/ARM64, macOS Intel/ARM64, and Windows x64/ARM64 assets plus SHA-256 sidecars.
- Embedded minimal dashboard served by the gateway with Overview, Devices, Sessions, Activity and Settings, Add Device enrollment flow, generated OS install commands, responsive layout, SSE refresh, and polling fallback.
- Dashboard APIs expose persisted devices, reconciled active work and bounded audit metadata. Audit records action/outcome/request IDs only, not stdout, file contents, enrollment secrets, or private keys.
- Static dashboard/bootstrap responses use no-store, nosniff and a restrictive Content Security Policy.
- Structured JSON tracing and Linux/macOS/Windows Rust CI definition.
- Isolated `scripts/smoke-v1.sh` verifies enrollment, signed authentication, MCP `exec`, identity persistence, and reconnect.
- Isolated `scripts/smoke-install-dashboard.sh` verifies dashboard assets, enrollment, the native installer flow, persisted config/identity, online reconnect, status, doctor, sessions API and activity API.
- `scripts/burnin-linux-user.sh` provides a real Linux user-service burn-in beside v0.3. On the Oracle ARM64 host it has verified non-login install/doctor, live MCP exec, fresh PTY I/O across a gateway restart, plus fresh PTY I/O and continued output from the same durable background job ID across an `infra-agent.service` controller restart.
- `scripts/burnin-remote-posix.sh` drives an already-enrolled remote macOS/Linux device through the gateway, requiring a newer connection generation after controller restart, fresh I/O on the same PTY, continued output from the same job ID, and optional same-PTY validation after a gateway restart.

### Current verification

On the Linux ARM64 development host:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace` — 54 Rust tests, including real PTY/job workers, inherited-nonblocking control-socket normalization, protected-identity metadata/file-fallback coverage, search/edit/hash, resumable transfer, structured system/network parsers, Git, container redaction and local lifecycle tests.
- `./scripts/smoke-v1.sh`
- `./scripts/smoke-install-dashboard.sh`

The current code head is green on Ubuntu x64, Ubuntu ARM64, macOS ARM64, and Windows x64 through formatting, strict clippy, and the full Rust test suite; Ubuntu x64 also passes both product smokes. macOS accepted control sockets are explicitly normalized back to bounded blocking I/O, closing the CI-observed BSD `EAGAIN` race. Windows ConPTY startup/query/close/shutdown lifecycle now passes the full CI suite and produces a native x64 acceptance artifact; real Windows-device validation remains the platform gate.

### Intentionally incomplete

- Production MCP/dashboard OAuth and a proper browser login/session. The gateway still refuses non-loopback binding by default unless an explicit development override is supplied.
- Device signing keys now use native OS credential storage when available (macOS Keychain Services, Windows Credential Manager, Linux Secret Service) with automatic v1→v2 identity migration and doctor visibility. Headless Linux without Secret Service intentionally falls back to a `0600` private identity file; eliminating or encrypting that fallback and broader Windows state-directory ACL hardening remain pending.
- Cryptographically signed release/update metadata. Bootstrap release installs currently verify SHA-256 assets; the steady-state updater and rollback chain remain to be built.
- `infra update`, staged binary replacement, health-check rollback, sanitized log export, and permission-specific doctor checks.
- Real-device reboot/sleep-wake testing across macOS and Windows remains. Linux user-service install/reinstall/controller-restart behavior is exercised on Oracle ARM64, and macOS ARM64 has a side-by-side native install with signed reconnect plus controller/gateway persistence burn-in; full machine reboot/reconnect tests are still required.
- Unified patch, line-range read, tail/watch and recoverable platform trash.
- Higher-level device-to-device transfer orchestration and throughput reporting. Binary device upload/download/resume exists at the MCP layer.
- Privileged helper and semantic desktop automation.
- Full dashboard device-detail/terminal/files/permission/diagnostic controls. The current dashboard is a functional control/status foundation, not the finished product.
- Gateway control-path liveness now uses 10-second protocol heartbeats with a 30-second stale timeout; any bounded RPC timeout immediately retires that connection so a dead socket cannot remain “online”. Agents refresh durable-work reconciliation after operations and heartbeats, and the real Linux burn-in requires active PTY/job/transfer counts to return to zero after cleanup.
- Linux PTY/job survival through a user-service controller restart is verified on Oracle by requiring fresh post-restart activity. The real Mac burn-in now passes with independent launchd workers + stable boot identity: the same PTY accepts fresh I/O and the same durable job emits later output after controller restart, and the same PTY remains usable after gateway restart. Submitted macOS worker wrappers self-remove after completion; post-burn-in validation shows zero worker launchd registrations/processes while both v0.3 and v1 controllers remain healthy. Windows CI is green and the x64 acceptance artifact is staged; Windows real-device validation remains pending.
- PTYs/jobs never claim to survive a full machine reboot; stale-boot entries are marked interrupted.
- Windows atomic replacement semantics need a platform-specific upgrade; overwrite may require destination removal before rename.

## Migration safety

All v1 development remains side-by-side. The deployed Node v0.3 remote-control service has not been modified, restarted, or migrated by this branch.
