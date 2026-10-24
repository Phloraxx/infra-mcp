# Infra onboarding v2 — product redesign decision and implementation blueprint

**Decision date:** 2026-10-24. **Status:** proposed for implementation and acceptance, *not deployed*. **Updated installation preference:** [ONE_COMMAND_SETUP.md](ONE_COMMAND_SETUP.md) makes verified one-command native setup the first milestone; the signed MSI described below is an optional later distribution channel, not a blocker. This supersedes the default first-run UX in [INSTALLATION_UX.md](INSTALLATION_UX.md) and the onboarding parts of [NEXT_STEPS.md](NEXT_STEPS.md). Retain older documents as dated implementation history.

## A. What went wrong: evidence rather than assumptions

The user's Windows trial yielded `ScriptBlock.Create(...)` on a null response (`InvokeMethodOnNull`). The referenced installer hostname did not resolve when rechecked on Oracle; a different canonical HTTPS gateway hostname did serve the expected PowerShell asset. A long script execution command concealed this simple prerequisite failure. On Windows PowerShell 5.1, safe web fetching also needs `-UseBasicParsing` after Microsoft's Dec. 2025 hardening. Existing fixes added that flag and an installer error, but **the default UX should stop asking people to execute downloaded scripts**.

Repository findings:
- The **dashboard asks for a Friendly name** before the user has installed anything (`apps/dashboard/index.html`) and can push `--name` into the command. The agent already defaults to OS `platform.hostname` if the override is absent (`crates/agent/src/main.rs`). The *input is optional in code*, but inappropriate in the primary UI. User-friendly **rename** belongs after discovery and can be performed without re-pairing.
- The dashboard's Windows command is long, script-centric, fragile across URLs, versions of PowerShell and operator copy/paste. The patch to use `Invoke-WebRequest -UseBasicParsing` is a **fallback fix**, not a long-term product solution.
- Windows `scripts/install.ps1` defaults to the GitHub `latest` release URL but **no published GitHub Releases** existed during the review. Windows x64 passing CI and a downloadable CI artifact are not a signed, published user installer.
- `crates/cli/src/startup.rs` Windows startup registers an ONLOGON `schtasks` task and ignores the result of `schtasks /Run`. `crates/cli/src/main.rs` then prints "Infra installed" without proving the service started and connected. This is **false success**, even if isolated `--no-startup` CI passes.
- The Windows CI smoke newly added to the branch tests signed enrollment from source binaries but explicitly uses `-NoStartup`: **it does not test Windows startup, upgrade, uninstall or real reboot**.
- The existing `devices` UI historically said online while RPC routing could report offline; even if not reproducible in this review, online truth must be read from the same authoritative routing state as dispatch and qualified by the connection generation.
- Mac reconnects have had heartbeat timeouts/reset events. Public WSS without compulsory Tailscale is still an unproven staging target. A pretty installer cannot fix unreliable transport.
- Existing build is **not zero work**: Rust gateway/agent, SQLite, Ed25519 identity, user startup, resumable PTYs/jobs/transfers, browser UI, OAuth, signed-updater groundwork and Android signed protocol are valuable and tested. A greenfield rewrite of those would reintroduce known bugs.

**Product diagnosis:** backend-first design leaked technical details into the owner journey, with ambiguous notion of completion and no cohesive local/remote pairing experience. **Rebuild the installation/pairing surfaces and contracts, not the device execution engine.**

## B. Promise and non-negotiable UX principles

1. **Normal desktop first use: download, open, approve, connected.** Zero terminal commands, zero gateway URLs, zero mandatory name entry, zero visible signing keys or device IDs. It is fine to offer advanced/headless CLI setup in a secondary view.
2. Device name is **detected locally** by the agent from its OS hostname; initial UI uses that name. Rename later through the dashboard as an *alias*, without changing the signed device identity or requiring reinstall. If hostname is missing, display a generic platform label; duplicate names receive disambiguation in the UI and cannot be routed ambiguously.
3. The device initiates pairing and shows a **short human-verification code**; the owner explicitly approves in their authenticated dashboard. The user does not carry a pre-issued expiring enrollment code through a lengthy download.
4. Installation, identity registration, network connection and optional permissions are four **independent resumable steps**. Restarting any step preserves completed safe state. An expired pairing request means *Retry pairing*, not *Uninstall and start again*.
5. **"Connected" means an accepted authenticated device connection with an actual fresh ping/status**, not "MSI exited 0", "task created", "identity file exists", or a cached registry row.
6. When something fails: identify which step, why in normal language, last successful checkpoint and one safe next action. Offer *Copy diagnostics* redacted of credentials, command lines, file content, addresses, pairing secrets and screenshots.
7. Default desktop transport is outbound verified TLS `wss://` on public port 443, no mandatory VPN or inbound client port. Tailscale can remain an optional expert fallback.
8. No stealth privilege escalation. Installer performs per-user installation by default; OS-native permission/security prompts are never hidden or bypassed. Desktop automation and Android accessibility require separate owner consent.

## C. Primary user journey (the contract)

### From the owner dashboard

```
My devices              [+ Add a device]
                     ┌──────────────────────────────┐
                     │ Add your computer            │
                     │                              │
                     │ [Download Infra for Windows] │
                     │  Not Windows? macOS · Linux  │
                     │                              │
                     │ Install it, then open Infra. │
                     └──────────────────────────────┘
                            [pending install]
                    ↓ signed installer launches
                     ┌──────────────────────────────┐
                     │ New device wants to connect  │
                     │                              │
                     │  Desktop-ABC · Windows 11    │
                     │  Compare this code with the  │
                     │  one shown on that computer. │
                     │           482 731            │
                     │                              │
                     │      [Approve device]        │
                     └──────────────────────────────┘
                    ↓ signed device connects / status probe succeeds
                     ┌──────────────────────────────┐
                     │  ✓ Desktop-ABC   Connected   │
                     │  Terminal · Files · Processes│
                     │  [Open device]   [Rename]    │
                     └──────────────────────────────┘
```

*Illustrative names and code only.* The owner can start with a direct download link, without first opening the dashboard; the installer opens the same pairing screen in the default browser. On an unauthenticated browser, the owner signs in before seeing/approving anything.

### Windows packaging decision

**Primary:** one **signed, per-user MSI** (`InfraSetup-x64.msi` initially, ARM64 separately), shipping both existing `infra.exe` and `infra-agent.exe`. Use WiX only at *build/release packaging* time; do **not** add a GUI framework or background runtime dependency. The MSI provides normal Install/Repair/Uninstall entry points and a final **Connect this PC** action that opens the browser-based approval flow via a small `infra pair` command implemented in the existing Rust CLI (no third executable). The agent manages the OS hostname, key creation and pending request.

MSI alone must never show "connected". The browser/CLI pairing view waits for an authenticated handshake; otherwise it shows retryable offline state. Do not depend on installer custom actions executing in an elevated SYSTEM desktop session. The first real Windows acceptance includes a standard user, a path with spaces and a locked-down PowerShell policy.

A standalone signed `InfraSetup.exe` is a legitimate alternative **only if tests show a single MSI download and standard browser handoff cannot meet UX/security constraints**. Avoid wrapping MSI in another EXE, a bundled Electron/Tauri setup UI, a Windows service and a tray app just to display progress.

**Signing:** Windows Authenticode-sign MSI and its binaries using a trusted publisher identity, verify signed release manifest/hashes at distribution, and implement rollback. **Critical feasibility gate:** Microsoft's Azure Artifact Signing public-trust service currently accepts individual developers only in the US/Canada; other individuals must evaluate an OV code-signing certificate from an eligible certificate authority, not assume the Azure service is universally accessible. The repository's own Ed25519 signed manifest/CI provenance helps integrity but does **not** substitute for Windows OS publisher trust. New signatures do not guarantee SmartScreen will be silent. Microsoft documents reputation-based warnings even for new validly signed binaries; do not instruct users to disable protection. Avoid publicly distributing unsignable dev packages as a final workaround. Also decide a release/publisher trust root *before* writing final marketing copy.

**Advanced only:** PowerShell native bootstrap remains for headless/dev/unattended managed environments, always as an explicit file fetch and signature verification before execution; do not display long `irm | iex` commands by default.

### macOS, Linux and Android

- **macOS:** same dashboard story with notarized, signed Developer ID `.pkg` (or appropriately signed app installer), existing Rust LaunchAgent, browser-based pairing, native-hostname default, separate TCC permission check. Keep default terminal out of the flow; headless CLI advanced.
- **Linux desktop:** per-user native package where packaging/runtime proves practical; first working iteration may be a simple *downloadable installer* with a short, clear fallback shell command. Servers retain idempotent documented CLI flags. Avoid installing 5 package formats initially.
- **Android:** existing signed Android foreground-service app; QR/deep link can open its pairing flow, but Android permission/Doze/boot limits must be represented honestly. No generic ADB/root/hidden background shortcuts.

## D. Pairing v2 protocol (reuse existing identity, no extra service)

Pairing approval is a separate state machine in the **existing gateway + SQLite** and the **existing native agent + CLI**; avoid a new broker or websocket hub. Preserve the current `/enroll` contract during migration for Android/legacy clients.

```
1. Agent on first run generates local Ed25519 key; persists pending request.
2. Agent POST /pair/request { key, platform, detected_hostname, nonce_proof, agent_version }.
3. Gateway stores pending request (random ID, key fingerprint, creation/expiry, minimal OS metadata).
   Responds with opaque pairing ID, short verification code, TTL.
4. Existing CLI opens user's default browser at /pair/<public-request-id>;
   browser auth is required and URL itself grants no approval rights.
5. Owner sees platform + OS hostname (untrusted label) and a short code.
   Installer/local pairing view displays SAME short code; owner must compare.
6. Owner approves via authenticated, CSRF-protected POST /api/pair/<id>/approve.
7. Gateway binds the public key to one stable device_id, marks pairing consumed atomically.
8. Agent proves possession of the same key and opens existing signed /agent WSS.
9. Gateway verifies a fresh handshake + status ping; only then "Connected".
```

### Essential user-context spike before building the installer

The existing agent **exits if it is not enrolled**, so the "install, then wait for approval" UX is **not** a small dashboard change. For minimal complexity, implement pending-pairing state in `infra pair` (existing Rust CLI), **not** a new always-running unsigned/unauthenticated agent. It generates and holds the protected pending identity, creates the short-lived request, waits or safely exits, and once approved writes the registered identity and launches/registers the agent in the intended interactive user context. If the owner closes the window/browser, a later run resumes the same pending key and renews the expired request. An unapproved device never gains command access.

The independent code comparison needs a **local source of truth** on the installing computer, not a code shown only in the remote dashboard. Before committing to MSI, prove one simple owner-visible approach on a fresh Windows machine: an MSI finish dialog with a local verification code, **or** a short-lived local setup-only browser view from the existing `infra pair` process, bound strictly to loopback with a per-run anti-CSRF token. Choose the smallest safe UX, not both. The owner-approved cloud page must show the corresponding code, and browser approval must not run with elevated installer privileges.

Windows Installer documentation warns against dialogs from custom actions and against elevated custom actions for browser launching. Prefer the normal post-install **Open Infra / Connect this PC** shortcut in the signed-in user's session over clever elevated MSI custom actions. A signed MSI installer and its pairing helper should be separately testable and restartable.

### Required security and recovery invariants

- Pairing request id is a high-entropy opaque locator but **not authentication**. Code is a human comparison factor, **not** a bearer authorization token. A malicious remote requester cannot obtain an approved device merely by knowing the id/code. Explicit authenticated owner confirmation is always needed.
- Human code must be derived/stored so server, owner page and **local installer** agree; include binding to pending public key/session and expiry; enforce one-time approval, input/CSRF and lockout where relevant.
- Rate-limit unauthenticated pair creation, bounded quotas per host/network and globally, short TTL (~10 min), garbage collect, protocol size limits. No device command dispatch until approved and authenticated.
- Safe idempotency for same request/public key across offline retries; no duplicate enrolled devices from double-click / reboot. A new request may replace an expired request with same local key while preserving unsent pending identity.
- Distinguish `request_pending`, `owner_approved`, `agent_authenticated`, `connected`. Approval alone is **not** an online device.
- Rename is a gateway display alias only; signed device ID and hostname retained separately. Duplicate names are disambiguated with OS/last-seen short ID, **never silently rerouted**.
- Old enrolled identities remain valid; migration is explicit and rollbackable, no forced re-enrollment. Legacy enroll endpoints can be retired only when Android/desktop legacy user journeys have migrated.

## E. Installation/recovery state machine, not scattered scripts

| State | Visible to owner | Recovery |
| --- | --- | --- |
| package_download | "Downloading Infra" | Retry URL, distinguish DNS/404/TLS/proxy |
| package_verify | "Checking publisher and integrity" | Stop on signature/hash mismatch; no unverified execution |
| files_installed | "Installing Infra" | Repair/reinstall preserving identities; previous install remains usable if staging failed |
| background_registered | "Setting up automatic startup" | Check actual Windows task user/context/path, don't silently swallow task-run error |
| agent_running | "Infra is running on this computer" | Local restart; no privileged scripts as default |
| pairing_pending | "Approve this device in your browser" | Renew pairing request, no re-download |
| paired | "Approved; connecting…" | Retry TLS/auth; preserve identity |
| online_verified | "Connected" | Verify current connection generation and status response |
| limited_permissions | "Connected · screen access needs permission" | Local OS settings/user grant; do not call entire device offline |
| interrupted | "Connection lost; trying again" | Bounded retries and last known reason; repair action when an authorized channel exists |

Each phase has a stable internal error code, human copy, safe retry/repair action and correlation ID. Installation should be **atomic enough** not to replace a working CLI/agent with only one binary: stage/verify both before activation; protect known-good version, and roll back if new agent fails readiness. New update flow follows same logic.

### Windows service lifecycle correctness

- Use Windows Task Scheduler for the signed-in user's per-user agent initially. Formalize task principal/logon type and privileges; source-of-truth task config is versioned and unambiguous.
- Do not ignore `schtasks /Run` failure, scheduled task query, process launch exit, or signed WSS handshake. Installation succeeds at the package layer while onboarding can still be "Needs action."
- Tests must cover actual task create/start/current user/path-with-spaces, second installer invocation, logoff/login and cold boot. Isolated `-NoStartup` smoke is useful but **cannot** certify installed startup.
- Avoid Windows SYSTEM services for a normal user desktop agent; privileged helper is a separate, explicit and later capability.
- `infra doctor --json` should differentiate binary installed, Task Scheduler installed/running, local PID, identity, DNS, TLS, signed challenge, backend route, version skew, permission and user-session state. Inability to inspect a privileged task is a distinct error, not "agent missing."

### Network and truthfulness

- Enforce **one public canonical control hostname** configured centrally in signed installer metadata, not retyped into commands. A single mistyped DNS name must lead to "Can't reach Infra" with DNS-specific guidance, never an opaque null-expression exception. Public agent path uses verified WSS.
- Keep an existing persistent WSS connection, priority Pong/control and reconnect with jitter. Don't introduce a second realtime service or new queue.
- Use the actual active connection registry and generation for both dashboard online state and tool dispatch. A stale last-seen database record is history, not proof of routability.
- A dispatch race may still occur after a fresh online snapshot. Return `device_disconnected_during_request` or `outcome_unknown` with request ID as appropriate, never quietly replay mutations.
- UI should differentiate "Computer offline", "Gateway offline", "Permission needed", "Pairing expired", "Local agent not running", "Network/TLS error" and "Revoked."

## F. Minimal UX surface and an explicit **not doing** list

**Only four top-level user intentions:** Devices, Sessions, Activity, Settings. **Device primary actions:** Open, Rename, Check connection, Update, Remove. Place expert controls under Device details or Settings > Advanced. Dashboard add flow is short and guided; previous technical code box goes to "Manual/headless installation."

Design desktop UI as a *static, accessible and responsive* incremental rewrite of `apps/dashboard/`. Reuse the embedded Rust-served HTML/CSS/JS and authenticated APIs. Do **not** migrate to React/Electron/Tauri/Next.js simply to make a pairing modal. It might justify reorganizing app.js into a few modules but no second API/Node server.

We will **not**:
- rewrite proven Rust PTY/jobs/files/transfer code;
- require Tailscale, SSH, manual port forwarding or gateway-address flags for everyday desktops;
- demand owner-chosen names, manual agent UUIDs or reusable secret tokens;
- ship an unsigned executable as "one-click" production software;
- hide OS permissions/SmartScreen/UAC or promise background powers Android does not have;
- build account/team/RBAC/analytics infrastructure for one owner;
- equate passing GitHub CI with a signed release or physical Windows reboot acceptance;
- decommission working legacy services before the new path is verified and rollback tested.

## G. Scoped workstreams, dependencies and exit gates

**P0: prove distribution and pairing before cosmetic UI changes.**

| Milestone / likely diff | Deliverable | Acceptance / failure tests |
| --- | --- | --- |
| **0. Define installer release trust** (`release.yml`, signed manifest, packaging) | One versioned Windows artifact + Linux/macOS release plan, source-of-truth URL and publisher/signature policy | Trusted signature, valid artifact names, no `latest` 404; corrupt/unavailable release fails before touching installed files |
| **1. Pairing v2 backend** (`gateway` and SQLite, Rust/Android compatibility) | Pending/approve/deny/status API; key-bound owner approval | Rate-limit, CSRF, TTL, retries, concurrent approvals, wrong key and replay; legacy enrolment still works |
| **2. Agent/CLI flow** (`agent`, `cli`) | Auto hostname, `infra pair`, browser launch, local pairing code, connection status, idempotent retries | Fresh and existing install with offline gateway; device key preserved; no extra name required |
| **3. Windows MSI** (`packaging/windows` + CI) | Signed per-user MSI with 2 native binaries, post-install browser approval and repair/uninstall | Real Windows standard-user install, paths with spaces, code-signing/SmartScreen notes, login startup, reboot, upgrade rollback |
| **4. Dashboard onboarding v2** (existing static assets) | Download / detected pending device / approve / connected live status; rename later | Plain-language errors, 390px mobile, keyboard/screen-reader, no displayed command in happy path, no secrets in URLs/logs |
| **5. WSS no-VPN acceptance** (staging ingress + tests) | Existing single gateway reachable by signed WSS 443 from each supported OS | Tailscale disabled on test client, DNS/TLS fail/recover, edge restart, revocation, live MCP tool routing |
| **6. Mac/Linux/Android parity + migration** | Appropriate signed macOS installer / Linux headless fallback / Android deep link, existing enroll compatibility | Stable identity, consent and background restrictions; original connected devices unaffected |
| **7. Controlled cutover** | First signoff via physical Windows + Mac + Oracle + Android tests | Real ChatGPT MCP use, warm latency, update/repair/rollback, 24h soak, no false online states |

Dependencies: 0 and 1 should begin in parallel; 1 before 2/4; 0+2 before physical 3; 3+4+5 before ordinary-user cutover. Publishing an MSI requires a trusted signing solution and is **not** implied by drafting this plan.

### Automated and manual acceptance suite

- Golden onboarding: **zero terminal commands, zero manually specified names**, one package download, one package open, one explicit approval; legitimate desktop appears with **actual OS hostname** and current connection state.
- Signed package denial: no release, bad DNS host, proxy/TLS outage, HTTP HTML at binary URL, empty body, wrong SHA256, bad signature, wrong architecture, revoked signing key, expired enrollment, stale request, rejected pairing, duplicate hostname.
- Repair: install twice preserves stable device_id; interrupted binary download, half-install, Windows Task registration denied, Task starts but agent quits, system tray/CLI not present, offline user, outdated version.
- Real Windows x64 with standard user and optional admin: Task creation/start/exit, reboot/relogin, ConPTY I/O, native credential store, signed key migration, update/rollback and uninstall.
- Mac: sleep/wake, LaunchAgent, TCC permission, restart, network switch. Linux server: headless non-login and service restart. Android: normal foreground/Doze/restricted scoped controls.
- Gateway: approval and actual online state agree with MCP routing; no `online` card when command cannot be routed; classify race accurately. Long-lived PTYs/jobs survive allowed disconnections.
- Usability research: test with a fresh Windows user unfamiliar with Infra, without them reading docs. Log where they hesitate, not just tool success. Screenshot/capture steps only with authorization. Make measured improvements.

### Proposed targets (not current results)

- 95%+ first-attempt success on supported fresh Windows machines **after** signing and publisher reputation are realistically established.
- Median under 2 minutes from downloaded package opened to a confirmed connected desktop on a healthy network; **exclude download duration or report it separately**, and measure user approval wait.
- Zero copy-paste commands and zero required user-defined fields for default desktop onboarding.
- Under 30 seconds after network returns for a previously awake agent at p95, **to be measured**, without pretending a shut-down PC or Android Doze has that guarantee.
- On failure the screen displays cause + next step and permits retry **without reinstall**.
- All CI and actual-device startup/upgrade paths tested before the button becomes the main production CTA.

## H. Rebuild vs preserve decision

| Component | Decision | Why |
| --- | --- | --- |
| Native Rust device agent, signed identity, operations, PTY/jobs/transfer | **Keep, fix lifecycle edges** | Valuable/tested core; reimplementation loses reliability |
| Rust gateway, SQLite, MCP, OAuth | **Keep, extend pairing APIs** | One control plane; no backend rewrite necessary |
| Dashboard visual and enrollment flows | **Rebuild the onboarding portion** | UI asks wrong questions, leaks flags/URLs, no coherent progress/repair |
| Windows script-first installer | **Replace as primary UX** | Fragile, poor native trust/security story; retain as advanced fallback |
| Release pipeline | **Rework packaging/signing/distribution** | A binary acceptance artifact is not a verifiable user install |
| Current Windows Task Scheduler adapter | **Harden and test** | Should preserve user context and prove startup instead of ignoring failure |
| Native desktop system app/tray, Electron, second gateway, QUIC | **Do not build now** | Unnecessary complexity before evidence |

## J. The *rest* of Infra must feel seamless too

Onboarding is the first visible failure, not the only product problem.

### Device selection and tools (the model's UX)

- A single canonical display label plus OS/type is shown consistently in the dashboard and MCP. The stable signed ID is internal. Use a short ID only to disambiguate duplicate device names; never require the model or owner to memorize UUIDs.
- Device discovery returns one authoritative snapshot: device state, capabilities, permission readiness, last_seen timestamp and route generation. Offline enrolled devices are listed but never advertised as controllable.
- For a command that targets a machine, **always** resolve the target against active routing immediately before dispatch. If the model uses a friendly name that matches more than one machine, return candidates instead of guessing.
- Keep coherent tools (`exec`, `files`, `search`, `terminal`, `jobs`, `copy`, `desktop`); make action schemas validate per-operation fields, instead of a giant optional-arguments bag. Error results explain the owner action (wake PC, approve pair, grant Accessibility, verify network), not raw exceptions.
- Common tasks should need one tool call. Provide bounded structured results (success, changed state, elapsed time, cursor/resume ID) rather than bulky duplicated logs. Don't replay side-effecting commands if the connection died after dispatch; `outcome_unknown` is a real state.
- “Do this on my laptop” should resolve a unique saved alias if one exists. “Do this on the computer” where multiple machines match must ask for a target. Never let a convenience default turn into accidental cross-device execution.
- Capability advertised by an agent isn't a grant: Windows/macOS interactive desktop permissions and Android temporary consent must be checked at use time.

### Normal daily product and failures

- A working setup requires **no app running in foreground** on Windows/macOS/Linux; agents reconnect when their OS/session allows it. The owner can open the dashboard or ChatGPT whenever needed. Don't pretend shutdown/offline endpoints can self-repair over a dead network.
- Overview: list computers by detected names, with recent accurate connection states. Device details should prioritize one primary action and show a short reason and next step when offline. “Repair this PC” is a **local** shortcut when the local executable exists; “Copy diagnostics” redacts private content.
- Persistent terminals/jobs remain attached to stable IDs, survive permitted controller reconnects and expose “Reconnect view” vs “Close process” distinctly. Files and copies must show progress, resumability, checksum and failures.
- First-use screens hide internal terms such as PTY, WSS, bearer token and Task Scheduler. They remain available under **Advanced diagnostics**.
- The dashboard should be resilient on mobile and accessible by keyboard; the usual path must not require a massive modal, horizontal scrolling or reading a 200-character command.
- No automatic global permission prompts for optional features during enrollment: connection success is enough to show “Ready”; advanced capabilities can ask for permission when the owner tries to use them.

### Performance and reliability without new systems

- One authenticated outbound persistent WSS per active device, heartbeat/control priority and bounded multiplexing. Use SQLite for metadata; no Redis, polling microservice, Kafka or custom QUIC stack.
- Run **warm connection** small-exec/read/terminal round-trip and transfer/reconnect p50/p95/p99 benchmarks on real Windows, Mac, Oracle and Android networks. Separate ChatGPT tool-call overhead from device execution. A fresh HTTPS health probe doesn't measure steady WSS latency.
- Annotate failure stage and recoverability. Correlate exactly one device generation, request ID, retry count and last-successful timestamp. Deduplicate idempotent requests, but not uncertain side effects.
- Before retiring the previous gateway/agent services, run an explicit canary on each device, preserve a rollback executable and unchanged signing key, then verify MCP commands after a real reboot/relogin.

## I. Sources, limitations and handoff

Current references:
- Windows PowerShell 5.1 parsing change: https://support.microsoft.com/en-us/servicing/os/windows/2025/12/powershell-5-1-invoke-webrequest-preventing-script-execution-from-web-content
- Microsoft SmartScreen reputation: https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation
- Microsoft current publisher signing eligibility: https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart
- Per-user MSI contexts: https://learn.microsoft.com/en-us/windows/win32/msi/installation-context
- Windows `schtasks` task-run behavior and limitations: https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-create
- Microsoft MSI custom-action security and avoiding hidden elevated UI: https://learn.microsoft.com/en-us/windows/win32/msi/guidelines-for-securing-custom-actions
- Apple Developer ID signing/notarization: https://developer.apple.com/developer-id/
- Optional GitHub release provenance via artifact attestations (not a replacement for publisher signing): https://docs.github.com/en/actions/concepts/security/artifact-attestations
- WiX packaging and current license/maintenance rules: https://docs.firegiant.com/wix/
- [Windows current acceptance](WINDOWS_INSTALL_ACCEPTANCE.md), [Canonical runtime reality](CURRENT_REALITY.md), [Existing UX principles](UX_PRINCIPLES.md).

**Implementation directive for future coding agents:** do not re-patch the old PowerShell happy path and call this redesign done. Begin with the pairing v2 state machine and an installable authenticated Windows release artifact; then implement the thin browser/dashboard UX. Preserve the older OAuth-worktree uncommitted modifications, current service routes, native keys and rollback installs. Conduct end-to-end acceptance before proposing replacement of a live device. Do not claim this design has been built merely because documentation has been committed.
