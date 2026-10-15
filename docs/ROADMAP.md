# Infra v1 roadmap

The roadmap is ordered to produce useful capability early while keeping the working v0.3 deployment available as fallback.

## Phase 0 — freeze the prototype

- [ ] Treat the live Node v0.3 system as compatibility reference, not the place for major new features.
- [x] Capture v1 product/architecture/protocol/tool/security context in this repository.
- [ ] Record reproducible v0.3 smoke tests before replacing behavior.

Exit: implementation can proceed without relying on chat history.

## Phase 1 — Rust foundation

- [ ] Create Cargo workspace.
- [ ] Add `protocol`, `gateway`, `agent`, and `platform` crates.
- [ ] Structured tracing/logging.
- [ ] Common error model.
- [ ] CI for formatting, clippy, tests, and build matrix.
- [ ] Gateway health endpoint.
- [ ] Agent connects to gateway over authenticated WSS.
- [ ] Device list/status through MCP.

Exit: Linux ARM64 agent connects reliably and is visible through MCP.

## Phase 2 — v0.3 capability parity

- [ ] one-shot `exec`
- [ ] files read/write/list/stat/copy/move/mkdir/delete
- [ ] process list/find/terminate
- [ ] durable background jobs with incremental output
- [ ] automatic reconnect/backoff
- [ ] pairing/revocation
- [ ] startup service support for Linux

Exit: v1 can perform every essential v0.3 operation on the Oracle test host without using the Node agent.

## Phase 3 — clear Desktop Commander advantage

- [ ] real PTY/ConPTY terminal sessions
- [ ] attach/detach/resume
- [ ] offset-based terminal reads
- [ ] line/byte-range file reads
- [ ] exact edit + unified patch
- [ ] efficient content/filename search
- [ ] tail/watch
- [ ] binary upload/download
- [ ] hashes and resumable transfer

Exit: development workflows such as Codex/REPL/dev-server sessions are materially better than Desktop Commander.

## Phase 4 — resilience and all primary platforms

- [ ] connection generation/supersession
- [ ] boot/agent instance identity
- [ ] reconciliation after reconnect
- [ ] macOS ARM64 agent + launchd install/startup
- [ ] Windows x64 agent + startup
- [ ] device-protected credential storage
- [ ] `infra status/reconnect/doctor/logs`

Exit: Linux, macOS, and Windows remain connected through ordinary reboot/sleep/network changes.

## Phase 5 — structured system modules

- [ ] system summary/disks/resources
- [ ] network listeners/port ownership/connectivity
- [ ] service abstraction
- [ ] Docker/container module
- [ ] Git module
- [ ] structured result pagination/filters

Exit: common operational prompts require fewer calls than raw-shell workflows.

## Phase 6 — privileged helper

- [ ] helper protocol and local authenticated IPC
- [ ] Linux elevated service/package primitives
- [ ] Windows service helper
- [ ] macOS helper strategy
- [ ] audit and policy separation
- [ ] install/uninstall/repair flows

Exit: selected admin operations work without running the main agent permanently elevated.

## Phase 7 — desktop control

- [ ] application/window discovery
- [ ] screenshots/regions
- [ ] accessibility/UI tree
- [ ] semantic find/click/type/wait
- [ ] mouse/keyboard fallback
- [ ] clipboard
- [ ] multi-display behavior

Exit: representative GUI tasks succeed reliably without coordinate-only scripts.

## Phase 8 — distribution and updates

- [ ] release binaries for primary platform/architectures
- [ ] one-command installers
- [ ] package installers where useful
- [ ] signed/authenticated update manifest
- [ ] staged update + health check + rollback
- [ ] `infra update`, `repair`, `uninstall`
- [ ] optional tray/dashboard

Exit: installation and recovery are simpler than Desktop Commander on all primary platforms.

## Phase 9 — benchmark and cutover

- [ ] run `BENCHMARK.md` matrix on same machines
- [ ] close every must-win gap
- [ ] run v1 as primary for normal work for a burn-in period
- [ ] stop Desktop Commander
- [ ] retain rollback path temporarily
- [ ] archive/remove legacy Node implementation only after successful cutover

Exit: Infra is the sole normal remote-control MCP.
