# Android agent: direct Infra v1 integration plan

**Status:** Design and repository context, NOT implemented direct-agent support.
**Baseline reviewed:** 2026-10-18; Motorola Edge 60 Stylus, Android 16/API 36, firmware W1VBS36.62-22-17-10. Firmware and identity details are illustrative; never hardcode them.
**Code starting points:** `apps/android-agent/` (imported working v0.1 APK source), `crates/protocol/src/lib.rs`, `crates/gateway/src/main.rs`, `crates/agent/src/main.rs`.
**Companion documents:** [wire adapter](ANDROID_PROTOCOL_ADAPTER.md), [security/acceptance](ANDROID_SECURITY_AND_TESTS.md).

## Goal and non-goals

Make the phone a **first-class, enrolled Android device** in the **existing Rust Infra v1 gateway**, discoverable from the existing `devices` MCP tool, while preserving user consent, Android sandboxing, battery efficiency and explicit capability reporting.

Target user flow:
1. Install a signed APK without requiring root, an unlocked bootloader, developer options, wireless debugging or a tethered Mac.
2. Open **Infra Phone**, choose **Connect to Infra**, supply a one-time code generated from the existing owner dashboard, and confirm device identity.
3. APK generates and protects its own signed device identity and starts an outbound secure agent connection to the existing gateway.
4. Gateway shows the device with `platform="android"`, stable ID, capabilities and presence; current ChatGPT MCP calls route to it like other devices.
5. Standard phone functions work over Wi-Fi/mobile data while permissions and foreground session allow. The user can stop, revoke or unpair visibly and immediately.

The **existing Oracle VM still hosts the gateway**, but it should no longer execute `phone-agent` commands, hold a phone-specific API session token, forward ADB or make inbound HTTP calls into the phone. **Android initiates one outbound WSS connection**; the gateway routes typed operations over that socket.

Non-goals for initial release: unrestricted ADB-equivalent shell; hidden remote control; access to private data of other apps; rooting; privileged package installation; bypassing screen lock or platform confirmation; guaranteed 24/7 phone hosting; remote execution of arbitrary commands from untrusted sources. Treat later shell, screen-capture and automation as separate opt-in projects.

## Verified baseline versus proposed future state

| Area | Verified on 18 Oct | Direct-agent target |
| --- | --- | --- |
| App | APK built and installed; project imported as source | Durable signed Android releases |
| Android OS | Android 16/API 36, locked bootloader | No unlocking/root required |
| Local API | Manual foreground service; loopback + Tailscale HTTP port 8765 | Retire private HTTP listener after migration |
| Authentication | Ephemeral bearer key stored by temporary Oracle helper | Per-device Ed25519 key, signed challenge, revocation |
| Connectivity | Oracle reached phone via tailnet; HTTP 401 without key and site 200 | Android -> gateway outbound WSS, no inbound phone port |
| API | Status, workspace read/write, selected-folder listing/read, static site | Infra v1 `status`/`system`/`files` subset; unsupported typed results |
| Enrollment | Not registered as Infra device | Uses gateway `/api/enrollments` + `/enroll` |
| ChatGPT routing | Indirect helper via Oracle `exec` | Same `devices`/`system`/`files` MCP tools |
| Screen/touch | Not present in imported v0.1 source | Explicit later consent/permission milestone |
| Background | Running foreground session was tested, not endurance | Tested recoverability, no uptime guarantee |

**Do not confuse the old HTTP helper with direct MCP integration. Do not claim the APK currently registers with the gateway.** Live legacy v0.3 and the working v1 gateway must remain untouched during this planning milestone.

## Architecture

```text
ChatGPT MCP client
       | existing authenticated Streamable HTTP
       v
Infra v1 Rust gateway + SQLite device registry
       |  /agent (outbound connection initiated by Android)
       |  WSS + existing challenge/signed_hello protocol
       v
Infra Phone Android app
  - ConnectionController (foreground service; retry/backoff)
  - IdentityStore (Ed25519 secret encrypted via AndroidKeyStore AES-GCM)
  - ProtocolCodec (exact Rust serde JSON framing)
  - AndroidDispatcher (allowlisted, permission-gated operations)
  - Workspace / SAF document grants / DeviceDiagnostics
  - User-facing pairing, connected-state, Stop and Unpair UI
       |
       +--> optional future Termux integration (separate consent + trust boundary)
       +--> optional future UI control (Accessibility + MediaProjection consent)
```

**Architecture choice:** Java/Kotlin Android app and native Android SDK APIs are a more reliable first step than embedding the desktop Rust agent binary. Directly embedding the Rust desktop binary would not confer Linux/ADB privileges or Android background exemptions. Reuse the **wire protocol** from Rust and verify byte-for-byte compatibility with test vectors before connecting production. A Rust-in-Android-JNI core can be revisited if it genuinely reduces duplicated protocol logic, but JNI/service lifecycle and cross-compilation have maintenance cost.

### Gateway compatibility and exact code anchors

- `crates/gateway/src/main.rs`: `POST /api/enrollments` (owner-authenticated code issuance), `POST /enroll` (device key registration), `GET /agent` (WebSocket), `authenticate_signed`, `handle_agent`.
- `crates/protocol/src/lib.rs`: `PROTOCOL_VERSION=1`, `DeviceAuthStart`, `AuthChallenge`, `DeviceHello`, `SignedDeviceHello`, `HelloAck`, `AgentRequest`, `AgentResponse` and `OperationResult`.
- `crates/agent/src/main.rs`: native connection lifecycle, signed authentication, heartbeats, reconcile behavior.
- `crates/agent/src/identity.rs`: enrollment payload and deterministic signing message.
- `docs/PROTOCOL.md`: authoritative protocol intent; Rust code authoritative for precise JSON and byte layout.

The gateway already accepts arbitrary platform strings in `DeviceHello`. **Do not add a new MCP endpoint or an alternate phone-only registry unless testing proves necessary.** The Android adapter can initially speak the existing protocol unchanged. Necessary gateway additions should be limited to capability-aware presentation and Android-specific structured action variants **only when** the requested function cannot fit existing types.

### Connection target: private/TLS first

- Use a configurable **WSS HTTPS gateway hostname with certificate validation**; do not hardcode private IPs, live tokens, ports, or home network details in app or Git.
- Pilot against an isolated v1 test gateway/registry before live enrollment.
- Existing owner-facing public dashboard ingress may block the agent endpoint; verify `/agent` WSS routing at the selected private gateway. Do **not** weaken public MCP authentication, disable certificate checks or expose a new public service to solve this.
- Tailscale is a convenient private network **transport** if installed and connected, not the agent protocol. Prefer Android-initiated WSS to an existing trusted HTTPS gateway; private tailnet IP is an optional route. Phone must continue to work if the MacBook is offline.
- Android VPN is owned by its Tailscale app, not by Infra Phone. Detect reachability and reconnect after network/VPN changes.

## Capability model: honest Android semantics

Existing Rust `AgentOperation` values can be handled selectively. On unsupported operations, respond `unsupported`, not a fake successful result. Gateway UI should indicate capabilities and permissions separately, rather than assuming all `files` or `exec` functions exist on Android.

| Stage | MCP tool / operation | Android behavior |
| --- | --- | --- |
| MVP | `devices` | gateway sees signed enrolled Android device and online/offline state |
| MVP | `system summary` / `status` | model, OS API, battery, charging, storage space, network status, app version; no sensitive unique ID |
| MVP | `files list/read` | app workspace; selected SAF tree; deny all out-of-scope paths |
| MVP | `files write` | app workspace only, atomic staged writes; selected tree writes after explicit permission |
| MVP | `transfer` | bounded chunked workspace transfers; incremental streaming; SHA-256 verify |
| MVP | `network tcp_check` | harmless connectivity diagnostics initiated by app, if supported |
| Later | `git`/`terminal`/`exec` | explicit **Termux or sandbox environment** with separate authorization; no impersonated adb/root |
| Later | `desktop` | optional screen capture and accessibility gestures, independent permissions, live indicator |
| Later | hosting | static private previews; migrate durable/public workloads to Oracle, not the battery-powered phone |
| No | containers, system-wide services, root shell | report unsupported unless explicit separately authorized integration exists |

Define capabilities as granular names such as `android.device_info`, `android.workspace.read`, `android.workspace.write`, `android.saf.read`, `android.saf.write`, `android.static_preview`. To preserve existing MCP routing, also advertise canonical parent capabilities (`system`, `files`, `transfer`) **only for implemented subsets**; dispatch each action with scoped checks. Do not advertise generic `exec` merely because the app runs Java.

## Storage, hosting, and development

**Storage**: app-private workspace is the default; user chooses SAF tree(s) for other documents. Android 11+ restricts choosing storage roots and the Downloads directory as a tree and forbids app-private directories of other apps. Never use `MANAGE_EXTERNAL_STORAGE` as a default escape hatch. Use URL/document IDs only inside the permissioned tree and reject ambiguous relative paths.

**Development**: app can edit/upload source/assets and preview static HTML/JS in its workspace. For Python/Node/compilers, evaluate a separate **Termux user session** exposed via a limited local integration with explicit consent and scoped working directories. Document exact IPC/auth, process lifetime and install prerequisites before enabling a terminal. Root is not required for a sandbox but Android's native app UID is not an ADB shell.

**Hosting**: for personal, short-lived previews only. The current v0.1 static site is readable by any tailnet peer reaching its port (HTTP 200 without the API key); this is not suitable for secrets. Move previews behind authenticated WSS/HTTP gateway routing, or require a per-session authorization token and disable the inbound listener. Do not run production websites, unattended databases or other long-running public services on a personal phone.

**Data**: avoid phone book/SMS, photos, microphone, location, clipboard, screen content and notifications in MVP unless user has requested them, an Android permission supports them, and the UI explicitly displays their access state.

## Android lifecycle and limitations

- Start a declared valid foreground service **from a visible user action**; show a persistent notification with Stop. Android 12+ generally forbids foreground-service launches from the background. Android 14+ requires matching foreground-service permissions/types. The existing prototype declares `specialUse`; document purpose precisely and re-evaluate if release-channel policy disallows it.
- Android 15 `dataSync` / `mediaProcessing` foreground service categories have quotas; do not reclassify a permanent socket as data sync to sidestep policy. Android 16 also applies quotas to jobs launched by foreground services.
- Foreground service does **not** guarantee that Android, Motorola power policy, or a VPN will never interrupt networking. Treat restarts/disconnects as normal; exponential backoff (e.g. 1,2,4,8... capped 60 seconds with jitter), reconnect on network availability, low-cost heartbeat processing, and no partial action replay.
- Boot handling: no silent boot start in MVP. If owner requests automatic reconnection, require explicit preference and verify Android background-start exemptions empirically; never claim guaranteed always-on operation or an invisible daemon.
- Screen capture via MediaProjection requires renewed user consent per session on Android 14+; Accessibility requires user enabling a separate service and must not become a covert control channel.
- Do not request unrestricted battery-optimization exemptions by default; evaluate only after measured failure.
- Android 16 local-network protections are opt-in now and evolving; regression test targetSdk upgrades.

## Migration from imported v0.1 to signed direct agent

1. Freeze current APK behavior as a **reference baseline**, not a final production architecture. Keep it installed and the temporary helper functional until direct-agent acceptance passes.
2. Build `IdentityStore`, `ConnectionController` and `ProtocolCodec` in the Android module.
3. Wire pair/unpair/connection-status UI to existing v1 enrollment API. Never store enrollment code after successful registration.
4. Start outbound signed WSS and implement `status`, `system summary` and workspace-limited `files`.
5. Verify MCP `devices` lists the Android device without Oracle helper; test revocation, stop and reconnect.
6. Add user-selected SAF file actions and bounded resumable `transfer` with exact action/scope gating.
7. Eliminate bearer-token temporary path and inbound port from new build **after** acceptance; delete/revoke old session helper key from Oracle.
8. Evaluate Termux development integrations and explicit remote UI control separately.

No live MCP gateway restart, production Cloudflare change, new public port or bootloader action is required for this planning commit.

## Prioritized work packages

### P0 — repository, identity, baseline tests (prerequisite)
- [ ] Add Android CI: Java compilation, lint, dependency hygiene, unit tests, release-signing separation.
- [ ] Build deterministic Rust/Java signing-byte vectors and JSON codec fixtures.
- [ ] Prove Ed25519 software key generation and AES-GCM wrapping via AndroidKeyStore; define recovery/unpair behavior for invalidated keys.
- [ ] Add explicit Android permission/capability mapping and contract tests.
**Gate:** tests pass without production gateway.

### P1 — first-class device and health (core objective)
- [ ] Implement existing `/enroll` flow, stable device ID, signed `auth_start -> auth_challenge -> signed_hello -> hello_ack`.
- [ ] Outbound WSS with TLS verification; replace stale connection by generation; handle `ping/pong` and reconcile.
- [ ] Build minimally-scoped `status` and `system summary` dispatch.
- [ ] UX: Pair, Connect, Stop, Unpair, permissions, last error, connection health, visible session indicator.
**Gate:** Android appears under `devices`, authenticated MCP `system` works, disconnect/revoke/reconnect behavior correct.

### P2 — actual data operations
- [ ] Wire `files` workspace + SAF scope, bounded read/write, SHA-256, atomic writes, path traversal tests.
- [ ] Wire resumable transfer protocol; limit disk/memory/parallel requests.
- [ ] Prefer gateway-proxied, authenticated previews to unauthenticated tailnet static listener.
**Gate:** roundtrip files across gateway with correct hash and permission denial elsewhere.

### P3 — development and utility
- [ ] Research and prototype optional Termux/local execution bridge under its own app identity.
- [ ] Add scoped app launching, bounded process status and diagnostics through Android APIs as appropriate.
- [ ] Build private preview tooling, logs/health report without credentials.
**Gate:** demo a safe source edit, preview and file transfer without ADB.

### P4 — interactive phone UI (separate product/privacy decision)
- [ ] Permission-gated Accessibility interaction and MediaProjection session capture.
- [ ] Explicit on-phone approval for screen-control sessions and consequential actions.
- [ ] Session-specific capability expiry, visible indicator, stop/revoke on-screen.
**Gate:** permissions denied/withdrawn instantly stop the operation; tests cover locked screen and redacted apps.

## Compatibility, testing and operational gates

Full test cases and release security controls are in [ANDROID_SECURITY_AND_TESTS.md](ANDROID_SECURITY_AND_TESTS.md). Minimum real-hardware acceptance includes Wi-Fi->mobile, Tailscale reconnect, Doze/screen-off, phone reboot, process kill, gateway restart, refused permissions, revoked key, signed protocol replay attempt and low-battery behavior. **No claim of reliable always-on service until endurance testing proves it.**

## Documentation and source references (reviewed 2026-10-18)

- Android foreground-service restrictions: https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start
- Service types and `specialUse`: https://developer.android.com/develop/background-work/services/fgs/service-types
- Android 16 work quota changes: https://developer.android.com/develop/background-work/services/fgs/changes
- Doze and App Standby: https://developer.android.com/training/monitoring-device-state/doze-standby
- KeyStore / AES wrapping: https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec
- SAF directory grants: https://developer.android.com/training/data-storage/shared/documents-files
- Android 14 projection consent: https://developer.android.com/about/versions/14/behavior-changes-14
- Android accessibility gestures: https://developer.android.com/guide/topics/ui/accessibility/service
- Local network protections: https://developer.android.com/privacy-and-security/local-network-permission
- Infra protocol/architecture: [PROTOCOL.md](PROTOCOL.md), [ARCHITECTURE.md](ARCHITECTURE.md), and executable Rust code noted above.
