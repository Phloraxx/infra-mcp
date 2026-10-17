# Infra v1 implementation status

This file records what exists in code. Product intent remains in the other design documents.

## 2026-10-15 — native control + product UX foundation

### Implemented

- Native Rust workspace with `protocol`, `gateway`, `agent`, `platform`, `local-state`, and `cli` crates.
- MCP Streamable HTTP gateway with compact `devices`, `exec`, `files`, `search`, `processes`, `jobs`, `terminal`, `transfer`, `system`, `network`, `services`, `containers`, `git`, and `desktop` tools.
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
- Local `config.json` and `runtime.json` state shared by agent and CLI. Runtime status records PID, connection state, stable device identity, generation, last successful connection, sanitized last error, and a non-secret credential-storage verification timestamp written only after successful signed gateway authentication.
- `infra` CLI: `status`, `doctor`, `install`, `repair`, `reconnect`, `update`, and `uninstall`, including JSON status/doctor output.
- Startup adapters implemented for Linux systemd user services, macOS LaunchAgents, and Windows logon Scheduled Tasks. Linux startup reconstructs the standard user-bus environment for non-login/remote shells, and idempotent reinstall/repair reloads and restarts the controller so a replaced binary actually becomes active. Linux uses `KillMode=process` so durable PTY/job workers outlive controller-only restarts; `infra uninstall` explicitly terminates persisted Infra worker trees to avoid leaking them.
- Native bootstrap installers: `scripts/install.sh` and `scripts/install.ps1`. Development/source mode is testable without releases; release mode verifies SHA-256 sidecars before installing binaries.
- GitHub tag-release workflow defined for Linux x64/ARM64, macOS Intel/ARM64, and Windows x64/ARM64 assets plus SHA-256 sidecars.
- Embedded minimal dashboard served by the gateway with Overview, Devices, Sessions, Activity and Settings, Add Device enrollment flow, generated OS install commands, responsive layout, SSE refresh, polling fallback, and private-mode browser authentication.
- Dashboard APIs expose persisted devices, reconciled active work and bounded audit metadata. In private mode, MCP requires the admin bearer credential while the browser exchanges that credential once for a short-lived HMAC-signed `HttpOnly; SameSite=Strict` session cookie; cookie-authenticated state-changing API calls also require the `X-Infra-CSRF` header. The admin credential is not stored in Web Storage. Audit records action/outcome/request IDs only, not stdout, file contents, enrollment secrets, or private keys.
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
- `cargo test --workspace` — 59 Rust tests, including real PTY/job workers, inherited-nonblocking control-socket normalization, protected-identity metadata/file-fallback coverage, search/edit/hash, resumable transfer, structured system/network parsers, Git, container redaction and local lifecycle tests.
- `./scripts/smoke-v1.sh`
- `./scripts/smoke-install-dashboard.sh`

The current code head is green on Ubuntu x64, Ubuntu ARM64, macOS ARM64, and Windows x64 through formatting, strict clippy, and the full Rust test suite; Ubuntu x64 also passes both product smokes. macOS accepted control sockets are explicitly normalized back to bounded blocking I/O, closing the CI-observed BSD `EAGAIN` race. Windows ConPTY startup/query/close/shutdown lifecycle now passes the full CI suite and produces a native x64 acceptance artifact; real Windows-device validation remains the platform gate.

### Intentionally incomplete

- Standards-compatible OAuth/token validation for an Internet-facing/public MCP endpoint is still pending. Private-mode access control is implemented: non-loopback binding requires a >=32-byte admin credential, MCP accepts that credential as a bearer token, protected dashboard APIs accept bearer or a signed HttpOnly browser session, and rotating the admin credential invalidates existing browser sessions.
- Device signing keys now use native OS credential storage when available (macOS Keychain Services, Windows Credential Manager, Linux Secret Service) with automatic v1→v2 identity migration. Real macOS ARM64 validation migrated the side-by-side v1 identity into `native_keyring`, removed the private key from `identity.json`, and reconnected successfully with signed authentication. `infra doctor` no longer reads secret bytes from the native store; it consumes a non-secret runtime proof written by the agent after a successful signed connection. Headless Linux without Secret Service intentionally falls back to a `0600` private identity file; eliminating or encrypting that fallback and broader Windows state-directory ACL hardening remain pending.
- Cryptographically signed release/update metadata. Bootstrap installs and the new steady-state updater verify release SHA-256 sidecars, but an independently signed/authenticated update manifest is still pending.
- `infra update` now stages verified binaries, requires the replacement agent to reconnect healthy, retains rollback binaries, and restores the previous agent on health failure. macOS/Linux replace the CLI synchronously; Windows stops/replaces/restarts the agent and uses a short-lived helper to replace the running CLI after the command exits. Real-device Windows updater acceptance, sanitized log export, and permission-specific doctor checks remain.
- Real-device reboot/sleep-wake testing across macOS and Windows remains. Linux user-service install/reinstall/controller-restart behavior is exercised on Oracle ARM64, and macOS ARM64 has a side-by-side native install with signed reconnect plus controller/gateway persistence burn-in; full machine reboot/reconnect tests are still required.
- Unified patch, line-range read, tail/watch and recoverable platform trash.
- Higher-level device-to-device transfer orchestration and throughput reporting. Binary device upload/download/resume exists at the MCP layer.
- Privileged helper and higher-level semantic desktop automation remain. The replacement-critical `desktop` foundation now includes display/window discovery, screenshots/regions, application focus, coordinate click, text typing, key/modifier input, and clipboard get/set on macOS and Windows; real-device permission/behavior acceptance remains.
- Full dashboard device-detail/terminal/files/permission/diagnostic controls. The current dashboard is a functional control/status foundation, not the finished product.
- Gateway control-path liveness now uses 10-second protocol heartbeats with a 30-second stale timeout; any bounded RPC timeout immediately retires that connection so a dead socket cannot remain “online”. Agents refresh durable-work reconciliation after operations and heartbeats, and the real Linux burn-in requires active PTY/job/transfer counts to return to zero after cleanup.
- Linux PTY/job survival through a user-service controller restart is verified on Oracle by requiring fresh post-restart activity. The real Mac burn-in now passes with independent launchd workers + stable boot identity: the same PTY accepts fresh I/O and the same durable job emits later output after controller restart, and the same PTY remains usable after gateway restart. Submitted macOS worker wrappers self-remove after completion; post-burn-in validation shows zero worker launchd registrations/processes while both v0.3 and v1 controllers remain healthy. Windows CI is green and the x64 acceptance artifact is staged; Windows real-device validation remains pending.
- PTYs/jobs never claim to survive a full machine reboot; stale-boot entries are marked interrupted.
- Windows update replacement now uses an agent stop/start cycle plus a post-exit PowerShell helper for the running CLI; real Windows/Rose acceptance remains before treating this as production-safe.

## Migration safety

All v1 development remains side-by-side. The deployed Node v0.3 remote-control service has not been modified, restarted, or migrated by this branch.

## 2026-10-16 — dashboard device/session milestone

- Current security commit `e5fd994` passed both CI workflows, including Ubuntu x64,
  Ubuntu ARM64, macOS ARM64 and Windows x64. Its fresh Mac binaries were downloaded,
  verified independently by SHA-256, switched only in the v1 burn-in installation,
  and successfully reconnected with Keychain-backed signed identity. After reconnect,
  the fresh non-interactive `infra doctor --json` reported all checks healthy,
  including credential-storage proof. Real Windows/Rose acceptance is still pending
  because that device is offline.
- Device detail tabs, live capability/work metadata, real agent status check and
  device-scoped sessions/activity now exist.
- Browser terminal uses the same PTY IDs/operations as MCP, with local xterm.js,
  offset reads, input serialization, resize, Ctrl+C, explicit close and detach on
  navigation. Browser refresh reattaches by URL. Ambiguous writes are not replayed.
- Job output/cancel, transfer status/cancel, session device/type filters and directory
  browse/bounded text preview are implemented. Full transfer/file-manager UX,
  local doctor/repair, permission and update UI remain unfinished.
- API acceptance extends the existing installer smoke: unauthorized/CSRF rejection,
  restricted operation surface, shared browser/MCP PTY identity/transcript,
  open/write/read/resize/close and bounded text preview without payload auditing.
- `scripts/test-dashboard-browser.cjs` is an optional Playwright gate using fixture
  API responses; it complements the real gateway/agent smoke rather than claiming
  a full real-device browser acceptance test.

### Milestone validation and private deployment

- Linux ARM64 strict clippy, all 56 workspace Rust tests, both real product smokes,
  and optimized gateway build passed.
- The browser fixture gate passed in Chromium, including 390 px mobile layout and
  CSP checks. It is now a dedicated GitHub Actions job using an isolated pinned
  Playwright development dependency.
- The Tailscale v1 acceptance gateway had an older manually launched process holding
  its port while the configured systemd unit repeatedly failed with address-in-use.
  Only that verified v1 process was stopped; the tested gateway was installed with
  a retained previous binary, and its existing user service now owns the endpoint
  with zero restarts. No v0.3 or Desktop Commander service was changed.
- The older private listener still accepted unauthenticated device-API reads. After
  the upgrade, unauthenticated reads return 401 and cookie mutations without CSRF
  return 403. Real Mac signed reconnect and cookie-authenticated browser API
  status/PTY open/write/read/resize/close passed; session reconciliation returned
  to zero afterward.
- Current-head GitHub CI remains to be observed after pushing this milestone. No
  Windows-device, machine reboot, sleep/wake or full replacement benchmark claim
  is implied by these results.

## 2026-10-16 — owner login, shared registry and HTTPS identity

- Fixed the signed-out UX: a dedicated login/loading screen keeps the app shell
  hidden until authentication succeeds. A missing session no longer appears as an
  empty device registry.
- The prior real deployments had two isolated v1 gateways/databases. The existing
  Oracle public-key registration was imported into the shared gateway without
  changing device identity or private key; its v1 config now targets that gateway.
  The shared registry reports both Oracle and the Mac online. The old isolated
  gateway/database is retained as an acceptance/rollback reference; do not rerun a
  harness that repoints the live Oracle agent without deliberately isolating it.
- Owner authentication is now a username/password account backed by a private
  Argon2id hash, an independent browser signing secret, bounded verification
  concurrency and rate limiting. Settings supports current-password-verified
  password changes with atomic persistence and previous-session invalidation.
- The built-in generated monochrome logo is bundled as the sign-in/sidebar image,
  PNG/ICO favicon and touch icon, including a dark-mode treatment.
- The authenticated dashboard is served over HTTPS through a dedicated hostname
  and existing Cloudflare Tunnel, with no new public inbound machine port. Both
  ingress and the gateway deny public MCP access. Password mode is distinct from
  future public MCP OAuth.
- Strict clippy, all workspace Rust tests, the password/auth HTTP smoke and the
  expanded installer/dashboard smoke pass on the Linux ARM64 host. Browser
  fixtures pass dedicated sign-in, wrong-password isolation and all existing
  device/session/file/terminal/mobile checks.

- A clean Chromium browser reached the actual public HTTPS deployment, signed in,
  observed both devices online, verified Secure/HttpOnly/SameSite cookies and image
  assets, opened a real Oracle PTY, sent input, refreshed/re-attached to the same
  session, closed it and signed out. The live gate reports
  `LIVE_PUBLIC_BROWSER_PASS`. Temporary QA resources are removed after acceptance.

## 2026-10-17 — macOS reconnect UX acceptance

- Re-ran the real macOS ARM64 remote POSIX burn-in against the shared v1 acceptance
  gateway. The same PTY accepted fresh input and the same durable job produced later
  output after a LaunchAgent controller restart; cleanup left no worker registrations
  or worker processes behind.
- `infra reconnect` now waits for a fresh runtime generation/PID/timestamp and a live
  `online` agent before reporting success, instead of returning while launchd is still
  bringing the controller back. The fresh CI-built macOS ARM64 CLI was SHA-256 checked,
  installed only into the side-by-side v1 burn-in, and verified on the real Mac:
  reconnect advanced generation 4 -> 5 and an immediate `infra doctor --json` was
  fully healthy with Keychain-backed signed-authentication proof.
- `infra repair` now uses the same online-after-restart acceptance rule, so repair no
  longer claims success while the replacement controller is still starting.
- Full machine sleep/wake and reboot validation on macOS remains an explicit cutover
  gate; this milestone does not claim either one.

## 2026-10-17 — replacement-critical desktop + updater batch

- Added a compact `desktop` MCP tool and agent capability with macOS/Windows implementations for permission readiness, displays, windows, JPEG screenshots/regions, focus, coordinate click, text typing, key/modifier chords, and clipboard get/set. The macOS permission probe runs inside the agent process (not the CLI) so TCC attribution is accurate; it reports Accessibility and Screen Recording readiness and the corresponding System Settings locations. Screenshot payloads are bounded to 5 MiB before base64 encoding inside an authenticated 8 MiB control-frame ceiling, and permission failures are surfaced explicitly.
- Added `infra update` with release SHA-256 verification, staged replacement, new-agent reconnect health checking and rollback retention. macOS/Linux apply both binaries synchronously; Windows updates/restarts the agent first and launches a bounded post-exit helper for CLI replacement.
- Linux ARM64 failure-path acceptance deliberately installed a checksum-valid but nonfunctional agent. The updater timed out the replacement, restored the previous binary, clears a systemd failed/start-limit state through the recovery path, restarts it, and requires the restored agent to be online before returning the update failure. `infra doctor` is healthy after rollback.
- Development was batched to avoid runner latency. The final local sweep passed formatting, strict clippy, all 59 Rust tests, `smoke-v1.sh`, `smoke-install-dashboard.sh`, and `smoke-dashboard-password.sh` before the batch was pushed for cross-platform CI.
- Remaining validation for this batch is intentionally platform-focused: native macOS desktop permissions/actions, Windows compile/runtime acceptance, and updater acceptance on real devices.

## 2026-10-17 — canonical primary-device acceptance

- macOS ARM64 was promoted from the side-by-side burn-in layout to the canonical `~/.local/bin/{infra,infra-agent}` + `~/.infra-mcp-v1` layout while preserving the same signed device ID and native Keychain credential. `io.phloraxx.infra-agent` now points only at the canonical paths. The legacy v0.3 helper was preserved as `infra-v0.3`, and its separate LaunchAgent remains loaded as rollback.
- Real macOS `infra update` acceptance passed from the canonical install: SHA-256 verified assets were staged, the replacement agent reconnected with the same identity, the CLI was replaced, rollback binaries were retained, and immediate `infra doctor` was healthy.
- Real macOS failed-update acceptance also passed: a checksum-valid but deliberately nonfunctional agent was rejected; the exact previous agent and CLI hashes were restored/retained, launchd recovered, the signed device reconnected, and `infra doctor` was healthy.
- The canonical Mac passed the automated safe cutover matrix for exec, file edit/read/hash, search, processes, system/network inspection, PTY, durable jobs, binary transfer, desktop permission readiness, desktop discovery and clipboard. The only desktop readiness gate is macOS TCC: Accessibility and Screen Recording are both currently denied; the agent reports both explicitly.
- A live acceptance-gateway restart preserved the same Mac PTY and durable-job transcript across gateway epoch replacement and automatic device reconnect.
- Oracle Linux ARM64 was likewise promoted to canonical `~/.local/bin/{infra,infra-agent}` + `~/.infra-mcp-v1` paths with the same signed device ID and a healthy systemd user service. Desktop Commander remains active only as fallback. The automated safe cutover matrix passes all ten Linux-relevant categories; Linux correctly does not advertise the desktop capability.
- Real Windows/Rose acceptance remains blocked by the device being offline. Full Mac sleep/wake and reboot/login acceptance remain intentionally untriggered remotely because those tests can strand access at an OS login/power boundary.
