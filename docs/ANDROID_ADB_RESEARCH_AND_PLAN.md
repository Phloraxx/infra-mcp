# Infra Phone: embedded ADB research and engineering decision

**Prepared:** 2026-10-23. **Status:** proposed architecture, not an implemented capability. **Target:** Android 16 (API 36), Motorola Edge 60 Stylus, Infra Phone Dev. This document records verifiable platform behavior, measured facts from the existing build, engineering judgments, and release gates. Read [the 2026-10-23 handoff](ANDROID_HANDOFF_2026-10-23.md) before editing Android code.

## Decision

**Do not recreate Shizuku inside Infra Phone yet.** Prototype a small, app-owned wireless-ADB client talking to the phone's *existing* `adbd` endpoint over authenticated local TLS. Integrate behind the current signed outbound Infra RPC and an **explicit, independently revocable local elevation session**. Do not add a separate MCP server, public port, root daemon, persistent hidden service, or unrestricted remote shell by default.

This is a hypothesis until tested on the owner's actual firmware. A native `app_process` + Binder helper comparable to Shizuku introduces another lifecycle, bootstrap, IPC, and security boundary; it is warranted only if the direct ADB implementation demonstrably cannot meet the needed operations. **Shizuku integration is a fallback**, not a prerequisite.

### What we are optimizing

1. One existing APK and one Infra device enrollment, rather than multiple permanently installed companion apps.
2. Minimal *additional* dependencies, APK size, resident memory, wakeups and setup steps. Reducing the number of apps does **not** by itself make the APK smaller.
3. Reliable read-only diagnostics and package/app workflows before optional advanced commands.
4. Visible and revocable owner permission. No escalation of the ordinary signed Infra agent merely because its gateway is authenticated.
5. Keep the existing Rust gateway, compact MCP tools, signed WebSocket protocol, scoped files and transfers unchanged unless a wire change is proven essential.

## Verified baseline and unknowns

- GitHub repository: `Phloraxx/infra-mcp`. The Android implementation and working history are on `feat/android-agent-plan`; `main` has older design-era documentation. See `docs/ANDROID_CURRENT_STATUS.md` for verified 2026-10-18 acceptance.
- Android app sources: `apps/android-agent/`; Java, Android compile/target SDK 35, debug suffix `.dev`, Gradle, OkHttp, BouncyCastle, signed Ed25519 enrollment and outbound WSS.
- Verified Android v0.3.0 capabilities: status, system summary, app-private workspace files, bounded transfer, small SenseGuide-specific `exec` allowlist and SenseGuide-only, on-device-consented Accessibility screenshot/tap/back. **Advertised `exec` is not a shell.**
- A newer **unversioned** Mac source directory `~/Development/InfraPhoneNext` contains locally built v0.3.1 changes: 60-minute local SenseGuide lease; user-approved *self-update* staging/review/installer; fix to missing/default `cwd` handling; updated version code. Gradle tests and debug APK build passed locally; installation and end-to-end self-update are **not verified**. **Do not lose these changes when synchronizing the repo.**
- Existing v0.3.1 debug APK measured at ~9.72 MB on Mac; its compressed DEX entries total ~8.43 MB. This **is a debug-build measurement**, not a release-build baseline or proof of size savings.
- ADB on the Mac is installed, but its device list was empty at the most recent inspection. Neither wireless-ADB pairing nor direct on-device ADB is verified on this Motorola. Check current device state before reporting success.
- Oracle working copy `/home/drvij/infra-mcp-v1-work` is on `feat/android-agent-plan` with unrelated in-progress gateway changes and local commits. **Do not overwrite, reset, rebase, commit or deploy that working copy as part of this design task.**

## Platform facts (not promises of implementation)

### ADB is a separate authority, not an Android runtime permission

A normal app runs under its own isolated app UID. Calling `Runtime.exec("sh")` stays in that sandbox; it does **not** become the Android `shell` UID. ADB privileges require a separate connection authenticated and authorized by `adbd`, or a separate helper actually started under `shell`/root. Root access, private data from arbitrary apps, bypassing keyguard and permission dialogs are not inherent capabilities of `shell`.

Android 11+ supports pairing and connecting over Wi-Fi. Pairing and connection are **different services and usually different ports**. The pairing code expires; the user must approve pairing on the device. The Android 16 target does **not** have Android 17's newer ADB Wi-Fi 2.0 behavior. After reboot, network transition, Developer Options change, power-saving, or a debug-service restart, the client must handle loss of connectivity; do not promise permanent ADB.

A *same-device* client can potentially connect over loopback to an authorized on-phone `adbd` listener; [LADB](https://github.com/tytydraco/ladb) documents this pattern and its split-screen pairing workaround. Device/firmware validation remains mandatory. Prefer a loopback-only endpoint rather than routing ADB through Oracle, exposing debugging on a tailnet, or opening a public listener. **Never scan for another device's adbd** as part of automatic discovery.

### Why not embed all of Shizuku?

[Shizuku](https://github.com/RikkaApps/Shizuku) starts a privileged Java service via `app_process` with ADB/root-provided authority, then provides Binder-mediated system APIs to client apps. Reimplementing it would entail bootstrapping that separate process, service authentication, Binder/API compatibility, lifecycle and restart handling, on top of the current Infra process. It does not eliminate the need to bootstrap an authorized `shell` context. Its own setup documentation notes that wireless-debug startup generally must be repeated after reboot.

The Shizuku code is Apache-2.0, but its repository reserves its brand/app identifier and certain image assets. Do not copy branding or entire codebase. If choosing it later, first prototype a minimal bounded helper and document why direct ADB is insufficient.

### Candidate library: libadb-android

[MuntashirAkon/libadb-android](https://github.com/MuntashirAkon/libadb-android) documents ADB pairing, authenticated connections and shell streams; the README currently illustrates artifact `com.github.MuntashirAkon:libadb-android:3.1.1` with cryptographic dependencies (including Conscrypt; review actual resolved graph). It explicitly states it **has not been security-audited**. Library is offered under Apache-2.0 OR GPL-3.0-or-later, and its README notes LGPL transitive-license considerations. Review complete dependency/license inventory, maintenance, pairing cryptography and device behavior before adopting; pin versions/hashes and do not fetch or execute arbitrary source at runtime.

**Prototype choice:** prefer reusing the library over writing ADB TLS/pairing cryptography from scratch *if* dependency audit, test results and size budget pass. If not, test a smaller maintained alternative; do not casually copy an unreviewed ADB stack.

### Android 16 / later local-network permissions

Android's Local Network Protections are **opt-in on Android 16** and expected to affect later releases. Their documentation specifically mentions local TCP sockets and mDNS/`NsdManager`. Plan permission and error handling for loopback and local discovery by the app's target SDK; verify on Android 16 now and revisit when targeting newer SDKs. Keep pairing port entry as a manual fallback to mDNS discovery failures.

### Installer caveat

Android's `PackageInstaller.SessionParams.setRequireUserAction` supports `USER_ACTION_NOT_REQUIRED` only when a list of installer ownership, package target SDK, permissions and platform conditions all hold. It is **not** a universal silent-install entitlement. Continue with owner-reviewed self-update and system installer approval until a separately tested, narrowly scoped improvement is justified. An ADB `shell` identity may perform some installation operations, but that power must not quietly become accessible to any signed remote request.

## Approaches assessed

| Path | Additional installation | Operational tradeoff | Decision |
| --- | --- | --- | --- |
| Mac via wireless ADB, reached through existing Infra Mac agent | None on phone | Mac and phone typically need compatible network; Mac must be online | **Immediate fallback and testing tool** |
| Embed a local authenticated ADB protocol client in Infra Phone | None | Pairing UI, port discovery, key management, extra crypto dependencies; availability tied to wireless debugging | **Prototype first** |
| Make Infra Phone a Shizuku API client | Separate Shizuku install | Mature privilege broker, but two apps and service startup | **Fallback if direct approach fails** |
| Rebuild Shizuku-style `app_process` broker inside Infra | No separate installed app, but extra privileged process/code | Harder startup, API compatibility, IPC/security and lifecycle | **Not MVP** |
| Termux / Android developer shell tools | More apps/setup | Processes remain confined to granted UID without separate ADB bridge | **Not a replacement** |
| Device-owner / root | Device provisioning or invasive setup | High risk/disruption for a daily phone | **Out of scope** |

## Proposed architecture / interfaces

```text
ChatGPT / owner dashboard
  -> existing Infra MCP tools (same endpoint, no new tools for v1)
  -> Rust gateway: existing authentication, audit and signed device routing
  -> Android GatewayAgentService, foreground/connected
       -> LocalActionPolicy: local owner lease + action type + bounded args
       -> AndroidAdbClient (optional feature; never auto-elevates on install)
            -> Android-approved pairing & TLS auth to local adbd (shell UID)
            -> bounded, cancelable subprocess-like ADB request/response
       -> existing non-elevated AndroidFilesOps / Transfers / SenseGuide controls
```

- Keep the existing `exec` family, and route **only explicitly marked Android ADB operations** after a new locally authorized mode is active. Do not quietly make the SenseGuide `exec` allowlist a generic shell. For MVP, prefer exact operations (e.g. version/UID, package metadata) over free-form shell strings. Add proper structured shell/terminal semantics **only after** separate tests, audit and consent policy.
- Never advertise an ADB capability until pairing, transport authentication and the **currently active local owner lease** all pass. Reconcile capabilities on connect/disconnect/expiry; a static `hello` capability flag is not evidence of actual privilege.
- Keep local `adb` host key separate from Infra Ed25519 enrollment identity. Generate/store key material securely, preferably via Android Keystore-backed storage when compatible with TLS APIs. Never transmit ADB private keys to gateway/ChatGPT or include them in exports.
- Local privileged-action lease: **one hour only when the owner explicitly opts in**, monotonic-clock bound, revoked on explicit Disable, unpair, agent Stop, reboot/process state invalidation. Do not confuse it with the existing SenseGuide-only screen lease. Expiry affects Infra's remote authorization, not the OS-level persistent ADB pairing; display both states.
- An active one-hour lease does not authorize an unlimited background shell or arbitrary package installs. Tier actions: read-only status without repeated prompts during lease, app/permission mutations require local confirmation. Default deny on unknown/ambiguous operation, lock-state restrictions as appropriate and loss of UI service.
- Close streams on timeout, stop, revoke, disconnect or process death; bound stdout/stderr bytes, max concurrent streams, input length and execution time. Keep existing connection generation/rpc IDs; never blindly replay an unknown-outcome privileged write.
- Avoid hidden APIs, global accessibility permissions, open localhost HTTP endpoints, Android root, and persistent unattended elevation in the first iteration.

### Owner UX: three simple states

1. **Standard connection:** Infra online with existing non-ADB actions. No developer option needed; safe fallback always available.
2. **ADB not connected:** "Connect advanced Android controls" → explain security and request Android Wireless debugging manually → pair with six-digit code/port (split screen or notification if the Android dialog closes) → test exact shell UID and show `Connected as shell`.
3. **ADB paired, access off:** show authorization state and a prominent "**Allow advanced controls for 1 hour**" button, with a visible "**Disable now**" action, expiry countdown and notification while active. Pairing can persist after local lease expires; access cannot.

Errors must distinguish `not paired`, `Wireless debugging off`, `port changed`, `local-network denied`, `transport/auth failed`, `owner lease expired`, `unsupported`, and `ADB service unavailable`. Never print secrets or raw pairing codes to notifications/logs.

## Implementation sequence, deliberately small

### P0 — Canonical source and regression baseline (blocker)

1. Make a clean worktree/branch from `feat/android-agent-plan`, without disturbing the dirty Oracle checkout or running agent.
2. Diff `apps/android-agent` against the newer Mac `~/Development/InfraPhoneNext`. Reconcile the v0.3.1 one-hour consent/self-update/default-cwd changes and docs **as source**, not APK artifacts; review for regressions and version/signing compatibility. Update stale historical docs with a short current-status pointer rather than rewriting history.
3. Run Android JVM tests + `:app:assembleDebug`, existing Rust `infra-protocol` tests, verify debug signature and package name. Record tested Android build/hash and **separately** actual phone version. Do not auto-install a new APK.
4. Record baseline APK bytes, DEX/native/dependency sizes, cold-start, idle RAM/battery and reconnect behavior. Build on current Mac/CI without relying on private absolute paths.

**Exit gate:** a clean, reproducible Android source tree in GitHub with no undocumented delta from the APK we intend to deploy.

### P1 — Time-boxed on-device ADB pairing experiment

1. Create a **small opt-in experiment inside the Android dev flavor**, no new MCP gateway call or Shizuku clone.
2. Audit `libadb-android` dependencies and license; pin artifact versions; compare APK size against P0. If native crypto or transitive dependency overhead is unreasonable, stop before wiring production paths.
3. Add pairing code + pairing-port UI, authenticated TLS key storage, status, loopback connect and a manual connection-port fallback. Never store the ephemeral pairing code. Prove `id` returns `uid=2000(shell)`, not the app UID, on the Motorola while Wireless debugging is enabled.
4. Test wrong/expired code, stale port, Wi-Fi toggle, network switch, process death, reboot, owner revocation and paired-but-off states.

**Exit gate:** phone-only pairing and *read-only* shell identity check work reliably, with no opened public ADB endpoint or unauthorized command route. If same-device ADB fails after bounded investigation, use Mac ADB now and evaluate Shizuku as fallback.

### P2 — Expose the minimum useful signed MCP capability

1. Add an `AndroidAdbClient` adapter behind current `GatewayAgentService` dispatch and a local `AdvancedAccessPolicy`. Use the existing family/API when possible.
2. Start with read-only structured probes: shell identity, OS version/SDK, package metadata and optional diagnostic stats. Preserve the old restricted SenseGuide commands.
3. One-hour on-device authorization, immediate revoke, bounded outputs/timeouts/concurrency, structured errors and audit metadata. No arbitrary remote shell or silent install in P2.
4. Verify gateway and phone permission state after reconnect, revoked pairing, unpair and lease expiry. Exercise denial cases through the **real MCP connector**, not only local unit tests.

**Exit gate:** no privilege leakage across the user-facing lease boundary, no regression in non-elevated files/transfers or signed authentication.

### P3 — Carefully selected mutations / optional shell

Only after P2 green: choose concrete actions that are genuinely needed (e.g. controlled APK install or selected app management). Add on-device per-operation approval and signed APK verification as appropriate. Introduce full interactive shell/PTY only if user value and test coverage justify it; reuse Infra stream/cancel/reconcile semantics, do not implement a second terminal protocol.

**Exit gate:** owner can see and stop a specific action; interrupted operations report uncertainty; all permission refusals fail closed.

### P4 — Reliability, size and release

Measure a 1-hour session, screen-off/Doze, background restrictions, reboot, mobile ↔ Wi-Fi, gateway restart, device force-stop, and short disconnections. Prove offline/default Infra remains usable when ADB is unavailable. Run negative security tests and sign a reproducible Android release with a **stable release keystore**; do not mix debug/release signing or uninstall the existing enrolled app to make a test pass.

Release only to the dev package first; retain rollback APK signed with the same key and ensure ADB/revocation does not mutate network, OAuth, gateway or live Rust services.

## Acceptance tests / definition of done

| Test | Required result |
| --- | --- |
| APK update safety | Same package and signer, increasing version code; Android owner approval path or explicitly proven self-update exception; no identity loss |
| Pairing | Correct ephemeral code succeeds, wrong/expired fails; TLS key pinned; pairing secret not logged |
| Actual privilege | `id` reports `shell` only via authorized ADB transport; default app `exec` remains sandboxed/restricted |
| Unauthorized/expired | No commands while locally disabled, past lease expiration, after reboot/stop/unpair, or on invalid RPC |
| Network | Loopback-only ADB target; clean disconnect/reconnect on changing ports and Wireless debugging toggles |
| Resource bounds | Timeout, cancellation, output cap, concurrency and transfer failure behavior tested |
| Connectivity | Agent WSS, workspace/transfer, SenseGuide gates and gateway audit remain unchanged |
| Rollback | Prior APK + enrollment recoverable; no forced uninstall; debug/release signing documented |
| Footprint | Record debug & minified release APK, memory, idle CPU/battery, startup; compare to P0, no arbitrary size claim |

## Go/no-go rules

- **Go direct ADB** if Motorola phone-only pairing, reconnect, least-privilege authorization and signed-MCP negative tests all pass with acceptable measured footprint.
- **Use Shizuku client as fallback** if direct ADB fails real-device tests or adds unacceptable crypto complexity; accept separate app and startup.
- **Consider a compact `app_process` helper last** only if a benchmarked use case needs Binder privileged APIs unavailable through shell services, and a separate trust model/maintenance cost is justified.
- **No-go** if setup requires secret extraction, hidden persistent privilege, disabling essential platform security, or reworking the production gateway.

## Primary sources / research references (reviewed 2026-10-23)

1. Android Developers: [ADB wireless debugging and pairing](https://developer.android.com/tools/adb) — pairing vs connection, mDNS, Android 17 Wi-Fi 2.0 distinction.
2. Android Developers: [Local network protection](https://developer.android.com/privacy-and-security/local-network-permission) — opt-in Android 16 behavior, raw socket/mDNS effects.
3. Android Developers: [Android Keystore](https://developer.android.com/privacy-and-security/keystore) — key storage guarantees and limitations.
4. Android Developers: [PackageInstaller.SessionParams](https://developer.android.com/reference/android/content/pm/PackageInstaller.SessionParams) — conditional user-action-free update rules.
5. Shizuku: [project and architecture](https://github.com/RikkaApps/Shizuku), [official setup and reboot caveats](https://shizuku.rikka.app/guide/setup/).
6. libadb-android: [README and documented warning, dependencies, license](https://github.com/MuntashirAkon/libadb-android).
7. LADB: [same-device ADB example and pairing UX](https://github.com/tytydraco/ladb).
8. Existing Infra repo: [Android current acceptance](ANDROID_CURRENT_STATUS.md), [security](ANDROID_SECURITY_AND_TESTS.md), [v0.3 development](ANDROID_V03_DEVELOPMENT.md), [signed protocol](ANDROID_PROTOCOL_ADAPTER.md), [repo policies](../AGENTS.md).

**Verification legend:** external platform/library behavior above is supported by linked primary documentation; repo acceptance is supported by existing dated logs; the proposed architecture, footprint and on-device Motorola behavior are engineering hypotheses pending P1 validation.
