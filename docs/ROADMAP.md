# Infra v1 roadmap

The roadmap is ordered to produce useful capability early while keeping the working v0.3 deployment available as fallback. Checked items mean the implementation exists in this repository; platform reliability still needs the acceptance tests in `RELIABILITY_UX_TESTS.md` before cutover.

## Phase 0 — freeze the prototype

- [x] Treat the live Node v0.3 system as compatibility reference, not the place for major new features.
- [x] Capture v1 product/architecture/protocol/tool/security context in this repository.
- [ ] Record reproducible v0.3 smoke tests before replacing behavior.

## Phase 1 — Rust foundation

- [x] Cargo workspace and protocol/gateway/agent/platform crates.
- [x] Structured tracing/logging.
- [ ] Finish a unified common error model across every tool family.
- [x] CI for formatting, clippy, tests, and cross-platform build/test matrix.
- [x] Gateway health endpoint.
- [x] Signed authenticated WSS-capable agent connection.
- [x] Persisted device list/status through MCP.

## Phase 2 — v0.3 capability parity

- [x] one-shot `exec`
- [x] files read/write/list/stat/copy/move/mkdir/delete
- [x] process list/find/terminate
- [x] durable background jobs with byte-offset incremental output
- [x] automatic reconnect/backoff
- [x] one-time Ed25519 enrollment/revocation
- [x] Linux systemd-user startup implementation
- [x] Linux ARM64 real-device non-login install/repair plus gateway/controller restart burn-in
- [ ] Linux real-device reboot/reconnect validation

A full reboot remains required before declaring Linux startup validation complete.

## Phase 3 — clear Desktop Commander advantage

- [x] real PTY/ConPTY terminal sessions
- [ ] explicit attach/detach naming (list/read/write already reattaches to surviving PTY workers)
- [x] offset-based terminal reads with bounded retained history
- [x] byte-range text file reads
- [ ] line-range file reads
- [x] exact edit with expected-match safety
- [ ] unified patch
- [x] efficient filename/content search with ignore/glob controls
- [ ] tail/watch
- [x] binary upload/download
- [x] SHA-256 hashes and resumable transfer offsets
- [ ] direct/high-throughput device-to-device transfer optimization

## Phase 4 — resilience and primary platforms

- [x] connection generation/supersession
- [x] boot/agent instance identity
- [x] job/PTY/transfer reconciliation after reconnect
- [x] macOS LaunchAgent startup implementation
- [x] macOS ARM64 real-device controller/gateway persistence burn-in
- [x] Windows logon Scheduled Task startup implementation
- [ ] real-device sleep/wake/reboot validation on macOS ARM64
- [ ] real-device reboot/reconnect validation on Windows x64
- [ ] device-protected credential storage
- [x] `infra status`
- [x] `infra reconnect`
- [x] `infra doctor`
- [x] `infra repair`
- [x] `infra uninstall`
- [ ] `infra logs` / sanitized diagnostics export

## Phase 5 — structured system modules

- [x] system summary/disks/resources/network-interface summary
- [x] network listeners/DNS/TCP connectivity; listener PID ownership where the platform utility exposes it
- [x] user/current-context service abstraction across systemd-user, launchd and Windows SCM
- [x] Docker/container list/inspect/log/start/stop/restart foundation
- [x] Git status/diff/log/branches/remotes/fetch/pull/push foundation
- [ ] structured result pagination/filters where current bounded results are insufficient
- [ ] privileged/system-wide service operations through the future privileged helper

## Phase 6 — privileged helper

- [ ] helper protocol and local authenticated IPC
- [ ] Linux elevated service/package primitives
- [ ] Windows service helper
- [ ] macOS helper strategy
- [ ] elevated-action audit and policy separation
- [ ] full-control install/uninstall/repair flows

## Phase 7 — desktop control

- [ ] application/window discovery
- [ ] screenshots/regions
- [ ] accessibility/UI tree
- [ ] semantic find/click/type/wait
- [ ] mouse/keyboard fallback
- [ ] clipboard
- [ ] multi-display behavior

## Phase 8 — distribution, dashboard and updates

- [x] release workflow for primary x64/ARM64 platform binaries and SHA-256 sidecars
- [x] macOS/Linux one-command bootstrap installer implementation
- [x] Windows PowerShell bootstrap installer implementation
- [ ] publish and validate a real release before using the bootstrap path as the default production installer
- [ ] package installers (`.pkg`/MSI/etc.) where useful
- [ ] cryptographically signed/authenticated update manifest
- [ ] staged update + health check + rollback
- [ ] `infra update`
- [x] basic `repair` and `uninstall`
- [x] functional embedded dashboard foundation
- [x] dashboard enrollment and generated install command
- [x] SSE device/session/activity refresh
- [ ] finished device-detail, terminal, file, permission, diagnostics and update UI
- [ ] production browser authentication/session
- [ ] optional tray application

## Phase 9 — benchmark and cutover

- [ ] run `BENCHMARK.md` matrix on the same devices
- [ ] close every must-win gap
- [ ] run v1 as primary for normal work for a burn-in period
- [ ] stop Desktop Commander
- [ ] retain rollback path temporarily
- [ ] archive/remove legacy Node implementation only after successful cutover

Infra is not ready for destructive cutover merely because individual checked features work. Reliability, install/reboot behavior and the benchmark remain release gates.
