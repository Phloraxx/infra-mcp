# One-command Infra setup

**2026-10-24 · Implementation started in the isolated feature branch; not deployed.** Pairing, setup, approval and aliases are implemented and locally validated. Real signed release download, initial short bootstrap, rollback and physical Windows/no-VPN testing remain open. Local browser handoff is implemented and Windows CI now passes signed owner pairing with automatic OS hostname; no consumer release is published. See [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).

This is the new near-term onboarding priority, refining [ONBOARDING_REDESIGN.md](ONBOARDING_REDESIGN.md). A signed MSI can be offered later, but is no longer the prerequisite for a usable desktop installer.

## Owner experience

**Copy setup command → Infra detects this computer → Approve pairing in browser → Connected.**

No required device name, gateway URL, enrollment code, port, VPN, manually chosen architecture or runtime dependency. The native agent reads the operating-system hostname. A later dashboard Rename action sets an alias without changing identity.

For Windows and Unix, use **two tiny bootstrap scripts** (PowerShell and POSIX sh) feeding **one shared native Rust setup implementation**. The scripts are only download/verification adapters, never independent lifecycle managers. The existing Rust CLI gains a simple setup operation that checks existing state, reuses signed identity, performs device-initiated pairing through the existing gateway, installs native per-user startup, and verifies an authenticated online response. Re-running it repairs problems without creating a duplicate device.

The dashboard hides command complexity behind **Copy Windows command**, **Copy macOS/Linux command**, and an optional **Download installer** action. The standard script must not use a pre-issued code that expires while software is downloading; owner approval happens after the device generates a local verification code. Explicit browser owner confirmation and a key-bound challenge remain required.

## Implementation contract

1. **Release/trust:** Current GitHub releases are empty. Publish signed, versioned binaries for supported operating systems before promising that the public installer URL works. An ordinary SHA-256 checksum served from the same host is an integrity check, **not publisher authentication**. Script bootstrap trust must be explicit: TLS on a trusted canonical domain, authenticated release metadata and a pinned signer where possible; a signed native distribution is the stronger first-install trust root.
2. **Preflight:** detect platform/architecture, live canonical host, network/TLS/proxy and per-user filesystem permissions before modifying an existing install. Missing releases, 404s and HTML bodies must produce clear failures, not an empty script or incomplete files.
3. **Atomic install:** stage and verify CLI and agent as one version, preserve old version and device identity, then safely activate. Roll back on failed readiness. Windows running binaries need deferred replacement, not unsafe overwrite.
4. **Pairing:** local CLI generates/preserves a pending signing key and short comparison code; authenticated owner approves in the existing gateway dashboard. Approval alone isn't proof of connection. Expiry/rejection can be retried without reinstallation.
5. **Native startup:** install, start and **check actual execution**, including Task Scheduler on Windows, launchd on macOS, systemd user on Linux. Do not ignore task launch errors or announce success after mere registration.
6. **Ready probe:** require fresh signed agent connection plus status through the gateway; otherwise show which phase needs action. Differentiate Installed, Pairing, Connecting, Connected, and Needs Repair.
7. **Repair/upgrade:** re-running setup detects an already enrolled machine, preserves one stable ID, fixes only failed steps and does not silently revoke/re-enroll. Keep explicit local diagnostics and an advanced headless mode.
8. **Visibility:** human error + recoverable next action for DNS, TLS, proxy, unsupported OS, release unavailable, verification failure, pairing expiry, startup denied, offline agent, locked files and rollback.

## Windows-specific rule

Windows PowerShell 5.1 needs explicit safe web parsing after the December 2025 security update. Never rely on a download piped blindly to a script evaluator as the *dependable* default. Download first, validate the result and authenticated source, then run the local bootstrap. Do not disable Defender, SmartScreen or execution-policy safeguards. Offer a verified native binary installer when scripts are blocked.

## Acceptance / sequence

**Order:** published test release → shared Rust setup/repair state machine → key-bound device-initiated pairing → thin verified PowerShell/POSIX bootstrap → streamlined dashboard → isolated CI and physical Windows/Mac tests → optional MSI/PKG later.

Test Windows PowerShell 5.1 and 7, Windows x64/ARM64, paths with spaces, standard-user permissions, no release, bad DNS, downloads truncated, expired pairing, repeated setup, startup denied, reboot/reconnect, identity preservation, repair and rollback. The Mac/Linux paths must meet the same observable contract with native startup adapters. Do not cut over the current working Oracle/Mac/Android agents or gateway until staging and rollback are proven.

**Not required:** Electron, Tauri, Node runtime, a new daemon or backend, a privileged service, a separate broker, custom UDP transport or a second installer logic tree.

## Readiness audit — what is resolved, and what still needs a proof

**Reviewed 2026-10-24. This is the definitive next-build checklist, not a claim that setup exists.** Enough is specified to begin small prototype work. The supported first-run experience cannot be declared production-ready yet.

### Decisions fixed now (no further architecture discussion required)

- **Scope:** Windows x64, macOS ARM64 and Linux ARM64 are first test targets; add architectures already supported by release CI only after packaging smoke tests. Android keeps its signed app and legacy pairing during migration.
- **Process:** Windows PowerShell or POSIX sh downloads/validates one versioned release; the *existing Rust CLI* owns setup and repair. No new GUI runtime, service broker, Node/npm or package manager prerequisite. Keep an optional signed native package route for strict script policies.
- **Mode:** per-user agent/startup by default, no elevation or blanket desktop permissions. Headless servers use explicit advanced mode. User-provided name, gateway URL and enrollment code are absent from ordinary setup.
- **Pairing:** CLI locally shows short comparison code and attempts to open the authenticated public dashboard in a browser. Approve only the matching pending public key. If browser launch fails, print a safe URL and instructions; **approval never depends on local browser success**. Never put credentials or pairing authorization in the URL. The device cannot execute commands until approved, signed and freshly connected.
- **Identity:** existing native protected identity is immutable across reinstall, repair, version upgrade and network migration. On fresh setup, pending key survives retry and code renewal. Owners can **rename an alias independently**; hostname/agent labels remain separate signed-identity metadata.
- **Network:** a single public HTTPS/WSS canonical origin configured through the release, validated certificate and signed agent authentication. Keep Tailscale optional and old production route intact until the non-VPN staging handshake works.
- **Outcome:** package installed, paired, startup running and currently connected are **separate statuses**, all with stable machine-readable diagnostic codes. An installer cannot declare success just because files exist or a Scheduled Task was registered.

### Concrete code gaps and their earliest safe tests

| Gap / implementation location | Smallest fix | Test demonstrating closure |
| --- | --- | --- |
| **First download authenticity** — release workflow, Windows/Unix bootstrap | Publish a real versioned release with correct asset names, pinned verification key and manifest. Explicitly state HTTPS trust for the script itself; hash sidecars alone do not authenticate publisher. | Missing/HTML/tampered assets never execute; old binary and state stay untouched |
| **Old installation writes config too early** — `crates/cli/src/main.rs` | Write new endpoint and identity metadata transactionally **after** validation or retain prior config and roll back on pairing failure. | A failed setup against an unreachable gateway leaves an enrolled working machine unchanged |
| **Pairing cannot wait for approval** — `crates/agent/src/identity.rs`, `crates/cli`, gateway store/API | Add key-bound request/approve/status/expiry with SQLite atomic consume; reuse existing local protected pending key. No extra daemon. | Double clicks, lost response, expired code, wrong key and request replay do not create a device or elevate trust |
| **Alias would be overwritten** — `crates/gateway/src/store.rs::record_connect` | Store user alias separately from agent-reported OS hostname; render alias with a deterministic collision disambiguator; route by stable signed ID. | Rename persists after reconnect, update and hostname change; duplicate alias never routes to wrong machine |
| **Windows task may fail silently** — `crates/cli/src/startup.rs` | Check Task Scheduler create/start and actual running process; fail with action-specific error rather than masking result. | Denied task registration, wrong principal and task starts-then-exits correctly show Needs repair |
| **No authoritative end-to-end ready signal** — gateway routing + CLI + dashboard | Mark Connected only after fresh signed WSS generation and a verified status roundtrip; route the device ID to that same live generation. | Approved device with no socket is Connecting, not Connected; stale socket cannot appear healthy |
| **Concurrent installers and binary rollback** — installer and Rust setup | Per-user state-dir lock, stage both binaries as one version, preserve previous build, idempotent migration and controlled Windows replacement. | Two concurrent launches result in one active instance and device ID; interrupted upgrade restores working version |
| **Canonical public route not proven for agents** — isolated staging | Test HTTPS/WSS/PKCE and signed agent without a client VPN, across OS/network switch/edge disconnect. | Real Mac + Windows signed status and MCP tool call via public 443, with rollback |
| **Real-device acceptance** — Windows/Mac smoke and owner checklist | Use clean standard-user installs and controlled service login/reboot cases; measure reconnect/diagnostic accuracy. | Zero manual names/ports, no false success, repair without reinstall, preserved identity and clear failure messages |

### Minimal implementation slices and exit conditions

1. **Slice A: repository/test fixtures.** Pin one canonical release endpoint, enumerate asset names, build unsigned *internal test* bundles, and write failure-injection tests. Choose script trust model explicitly. No public promise while release key/signature is missing.
2. **Slice B: pairing and local state.** Make new SQLite pairing API + Rust pending key and `infra setup` resume semantics; test same-key retries and owner authorization. Retain legacy `/enroll` and original Mac/Android identities.
3. **Slice C: deterministic native setup.** Connect staged binary activation, native startup and true gateway-ready checking. Add state locking, rollback, repair/status, Windows PowerShell 5.1 failure tests.
4. **Slice D: dashboard and distribution.** Show one per-platform Copy command and independently tracked pairing/ready progress; publish properly authenticated release assets once key provisioning and verification pass. No mandatory name field.
5. **Slice E: non-VPN staging and field test.** Existing gateway on separate staging port/database, confirmed actual HTTPS tunnel ingress, authenticated public WSS and MCP, physical Windows logon/reboot and Mac network switch. Confirm rollback before cutover.

### Explicit unresolved external gates (not solved by planning)

- **Release trust root:** owner-controlled Ed25519 signing key and publisher/installer bootstrap trust choice must be provisioned. A script retrieved from a compromised endpoint can forge its own verification procedure; don't claim end-to-end publisher assurance from a remote script alone.
- **Ingress:** verify who controls the canonical public hostname, tunnel route, DNS, OAuth callback and WSS timeout policy. A `200 /healthz` is not an agent acceptance test. Never rewire the live origin without the owner's operational approval and a rollback route.
- **Actual Windows machine:** standard-user startup, network/policy, real Task Scheduler, credential manager, reboot and update rollback need owner-side acceptance. CI `-NoStartup` is insufficient.
- **Definition of supported user group:** default to the single owner and their authorized devices; do not design multi-tenant accounts, delegated approvals, public app store distribution or a privileged helper until explicitly needed.

### No-go conditions

Do not declare the redesigned installer finished, merge/deploy a new bootstrap as the normal path, or retire existing service agents if any of these are true: no authenticated downloadable release; setup changes an existing identity on ordinary retry; only CI proves Windows startup; a pairing request can be replayed by a different key; a broken agent reports Connected; a failure leaves the previous installation unusable; Tailscale is still needed on the client for the advertised default.

**Planning is sufficiently concrete to start Slice A and B. Real trust provisioning, actual Windows lifecycle and non-VPN staging are execution gates, not reasons to invent a more elaborate architecture.**
