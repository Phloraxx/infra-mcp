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
- [ ] Linux real-device reboot/reconnect validation (canonical Oracle install is otherwise accepted)

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
- [x] Windows x64 CI ConPTY lifecycle and native acceptance build
- [ ] real-device sleep/wake/reboot validation on macOS ARM64 (canonical install/update/rollback/gateway-reconnect accepted)
- [ ] real-device reboot/reconnect validation on Windows x64
- [x] native device signing-key storage through macOS Keychain Services, Windows Credential Manager, and Linux Secret Service when available, with automatic legacy migration
- [ ] eliminate/encrypt the explicit headless-Linux private-file credential fallback and finish Windows state-directory ACL hardening
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

- [x] application/window discovery foundation on macOS/Windows
- [x] screenshots/regions foundation on macOS/Windows
- [ ] accessibility/UI tree
- [ ] semantic find/click/type/wait
- [x] coordinate click, text typing and key/chord fallback foundation on macOS/Windows
- [x] clipboard get/set foundation on macOS/Windows
- [ ] multi-display behavior

## Phase 8 — distribution, dashboard and updates

- [x] release workflow for primary x64/ARM64 platform binaries and SHA-256 sidecars
- [x] macOS/Linux one-command bootstrap installer implementation
- [x] Windows PowerShell bootstrap installer implementation
- [ ] publish and validate a real release before using the bootstrap path as the default production installer
- [ ] package installers (`.pkg`/MSI/etc.) where useful
- [ ] cryptographically signed/authenticated update manifest
- [x] staged update + agent reconnect health check + rollback foundation on macOS/Linux/Windows
- [x] `infra update` foundation on macOS/Linux plus staged Windows CLI helper; real Windows updater acceptance remains
- [x] basic `repair` and `uninstall`
- [x] functional embedded dashboard foundation
- [x] dashboard enrollment and generated install command
- [x] SSE device/session/activity refresh
- [x] private-mode MCP bearer authentication + signed HttpOnly dashboard session/CSRF boundary
- [x] dedicated owner password sign-in, Argon2id, rate limits and session-rotating password changes
- [x] branded HTTPS dashboard hostname, local logo/favicon assets and public MCP isolation
- [ ] standards-compatible public OAuth/token validation and production TLS edge
- [x] device detail tabs, shared browser/MCP terminal and session controls
- [x] file directory browsing and bounded UTF-8 preview
- [ ] file upload/download/mutation, full doctor/repair, permission and update UI
- [ ] optional tray application

## Current cutover scope

For the owner-defined RDC replacement scope, GUI automation is not a release gate. macOS Accessibility, Screen Recording/audio capture, screenshots, click/type/key input, and richer desktop interaction are optional capabilities. Required cutover acceptance is limited to remote shell/exec, files/search, processes, PTY, durable jobs, binary transfer, structured system/network/service operations, canonical startup/reconnect, updater rollback recovery, and real-device reboot/reconnect reliability.

## Phase 9 — benchmark and cutover

- [ ] run `BENCHMARK.md` matrix on the same devices
- [ ] close every must-win gap
- [ ] run v1 as primary for normal work for a burn-in period
- [ ] stop Desktop Commander
- [ ] retain rollback path temporarily
- [ ] archive/remove legacy Node implementation only after successful cutover

Infra is not ready for destructive cutover merely because individual checked features work. Reliability, install/reboot behavior and the benchmark remain release gates.


## Android first-class agent extension (separate track, scoped plan 2026-10-18)

Full plan: [ANDROID_AGENT_PLAN.md](ANDROID_AGENT_PLAN.md). This section tracks the **Android project**, not the already-working desktop/Oracle agent implementation.

- [x] Import the buildable v0.1 Java Android source into `apps/android-agent/` and document current limitations.
- [x] Research the Android service, permissions, Keystore/SAF and network lifecycle constraints; write protocol/security/acceptance plans.
- [ ] Add clean Android CI and cross-language Rust/Java signed-auth golden vectors.
- [ ] Android `/enroll` plus signed outbound `/agent` WSS without an Oracle helper.
- [ ] First-class stable Android `device_id`, `devices` MCP discovery, revoke, reconnect, connection-generation handling.
- [ ] Implement Android `status` / `system` within sandbox and accurately scoped capability list.
- [ ] Add permission-gated `files`/`transfer` workspace and SAF operations.
- [ ] Remove temporary Oracle bearer-token bridge and unauthenticated inbound 8765 after acceptance.
- [ ] Separately investigate Termux-based development runtime; no generic exec/PTY without a real scoped implementation.
- [ ] Separately investigate on-demand Accessibility/MediaProjection UI control with visible consent and denial tests.
- [ ] Real phone / Android 16 lifecycle, sleep, network switching, gateway restart, revocation, transfer and energy acceptance.


### Android v0.2 implementation checkpoint (2026-10-18)

- [x] Ed25519 device identity encrypted at rest via AndroidKeyStore-wrapped AES-GCM and one-time code enrollment client.
- [x] Implement outbound WebSocket protocol client, signed hello, heartbeat and minimal status/system dispatcher in Java.
- [x] Install v0.2 as a separate `.dev` package; build Android and pass Java/Rust golden vector tests.
- [ ] **Verify** real signed enrollment and first-class device registration end to end against an isolated Rust gateway.
- [ ] Verify real private WSS without USB/debug reverse tunnel; ensure production ingress accepts `/agent` without relaxing owner/MCP auth.
- [ ] Add scoped Android `files`/`transfer` operations and tests.
- [ ] Real-device sleep/network/permission/revocation and reboot acceptance.

### Android scoped workspace progress

- [x] Implement and build-test non-destructive Android `files` RPC for app-private workspace; include path traversal/symlink denial JVM tests.
- [ ] Confirm `devices`, `system` and signed `files` roundtrip through an actually enrolled phone on an isolated gateway.
- [ ] Add user-selected Storage Access Framework tree handling to signed `files` with explicit URIs and permissions.
- [ ] Add resumable, hash-checked signed `transfer` and test large binary uploads/downloads.

### Android signed endpoint acceptance (verified 2026-10-18)

- [x] Enroll the Motorola phone into the existing deployed Rust Infra v1 registry using one-time code and Ed25519 signed WebSocket.
- [x] Confirm first-class Android shows **online** in native Infra-MCP `devices` with `status/system/files` capabilities.
- [x] Verify public `https://infra.drbijoy.in/` health, verified TLS, `/agent` websocket upgrade and unauthenticated enrollment rejection.
- [x] Verify live MCP `system summary` and scoped `files` list/write/read/hash/exact_edit/stat/mkdir using the phone device ID, **not Oracle helper or ADB forwarding**.
- [x] Remove temporary Tailscale-only test proxy; preserve existing tailnet routes.
- [ ] Test mobile data with Tailscale VPN disabled, Wi-Fi/mobile handoff, screen-off/Doze, service restart and revoke.
- [ ] Implement signed `transfer`, user-approved SAF folder access, better diagnostics, CI Android device tests.
- [ ] Decommission old per-session bearer HTTP helper and inbound local/tailnet listener after separate migration signoff.
- [ ] Explore Termux sandbox and explicit Accessibility/MediaProjection controls separately.

### SenseGuide diagnostic bridge (2026-10-18)

- [x] Add a user-approved import Activity in Infra Phone Dev for a single bounded SenseGuide JSONL report.
- [x] Require trusted SenseGuide provider, one-time URI read grant, explicit on-device Import confirmation, 64 KiB cap and valid report schema.
- [x] Build/test Infra Phone Dev and SenseGuide experimental debugging source without changing stable SenseGuide.
- [ ] Verify the complete phone-UI report export/import and signed MCP `files` retrieval using an actual running test app.
- [ ] Implement binary transfer and user-confirmed APK installation, plus replay fixtures in SenseGuide.

### Infra Phone v0.3 SenseGuide development (2026-10-18)

- [x] Implement scoped, SHA-256-verified binary `transfer` upload/download with <=128 KiB chunks, retries, cancel and bounds.
- [x] Implement owner-approved `apks/senseguide-debug.apk` installation intent with package check and standard Android installer confirmation.
- [x] Implement `exec` as exact allowlisted SenseGuide development operations; **not** general OS shell/Termux.
- [x] Implement user-enabled Accessibility service, five-minute on-phone consent, foreground-SenseGuide guard, bounded screenshot/tap/back; no clipboard or global app control.
- [x] Implement opt-in Android boot receiver and START_STICKY recovery; explicit limits apply to locked state and OS background restrictions.
- [x] Compile v0.3 Android unit tests and debug build locally.
- [ ] Verify signed transfer roundtrip, app status/request-launch, and installer UX on unlocked phone.
- [ ] Verify five-minute SenseGuide-only permission screenshot/tap; no capture of other apps.
- [ ] Validate actual screen-off, mobile/Wi-Fi changes, boot and gateway restart; restore connectivity on user interaction after app update as Android requires.
- [ ] Investigate optional Termux-backed command environments only with separate consent and owner-controlled containment.

### Android v0.3 live verification completed (2026-10-18)

- [x] Real phone v0.3 reconnection, registry capabilities, `exec` status.
- [x] Binary two-chunk upload, duplicate retry, SHA-256 commit, two-chunk byte-exact download.
- [x] Reject transfer path escape and unrestricted `exec`.
- [x] Stage complete 12,451,391-byte SenseGuide Diagnostics APK over signed Mac-to-phone MCP without ADB; independent file-hash match.
- [x] Restricted `request-install` returns owner review notice, with no silent installation.
- [ ] Confirm user approval via Android APK installer and separate test-app package checks.
- [ ] Enable and verify temporary Accessibility control by explicit on-device user action.
- [ ] Stress reconnect after screen-off/mobile network change/reboot; fix omitted-`cwd` ergonomics and completed transfer lifecycle polish.
