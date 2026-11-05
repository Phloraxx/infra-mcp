# Android agent: security, validation and cutover gates

**Scope:** Owner-controlled Motorola/other Android devices enrolled in existing Infra v1 gateway.
**State:** Planning checklist only; current imported v0.1 APK has NOT implemented native Infra enrollment or signed WSS.
**Related:** [Android plan](ANDROID_AGENT_PLAN.md), [protocol adapter](ANDROID_PROTOCOL_ADAPTER.md), [Infra security model](SECURITY_MODEL.md).

## Threat model

| Threat | Primary control | Required negative test |
| --- | --- | --- |
| Rogue peer contacts phone | Outbound-only WSS; eventually remove unauthenticated inbound site/API | No public, Wi-Fi or tailnet listening socket in target release |
| Gateway/device impersonation | TLS hostname check, one-time code, per-device Ed25519 signed challenge | Self-signed cert, wrong server, bad key, expired/replayed challenge fail |
| Reused pairing code | Gateway code expiry and atomic consume, pending request UUID | Same code cannot enroll a second device |
| Stolen phone or agent key | AndroidKeyStore-wrapped seed, gateway revocation | Stolen copy of prefs/files unusable; revoked key cannot reconnect |
| Accidental/malicious MCP command | Capability and permission checks at dispatch, per-operation scopes, consequences UI | A request from owner gateway cannot read disallowed files or silently capture screen |
| File traversal | Canonical workspace containment, SAF URI permission boundary | ``..``, symlinks, encoded traversal, malicious document names rejected |
| Replayed write after network loss | One request ID, idempotence classification and `outcome_unknown` | No duplicated create/write/delete on stale socket |
| Oversized data | Bounded frames/chunks, concurrency, disk quotas, transfer hash | High-volume frames/repeated tiny chunks cannot exhaust phone |
| Covert control | Visible foreground service, Stop/Unpair, consent for sensitive APIs | Permission revocation/stopping service drops control immediately |
| Clipboard/notifications/other app data | No permissions by default; separate narrowly scoped consent | Data not accessible from baseline install |
| Private data in diagnostics | Metadata-only audit, redaction, no keys in app logs/backups | Inspect logs, crash reports, generated APK and Git for secrets |
| Untrusted development process | Termux/plugin subprocess isolated, explicit user approval, no privileged bypass | Malicious script cannot escape Android's app sandbox via Infra |
| Malicious update | Build/sign with stable release key, reproducible provenance, controlled rollout | Tampered APK rejected; signing-key backup/rotation plan exists |

Trust boundaries:
1. ChatGPT client -> authenticated MCP gateway (owner session).
2. Gateway -> enrolled Android signing identity (TLS + signed challenge).
3. Android agent -> OS sandbox and SAF grants (gateway authentication is **not** Android permission).
4. Optional Android Accessibility/MediaProjection/Termux integrations have **additional**, short-lived grants. Never auto-enable during pairing.
5. Oracle server and Android phone are separately compromisable; reducing phone inbound listeners lowers exposure but does not eliminate risks from a compromised gateway.

## v0.1 baseline issues, no claim of resolved security

- Current `AgentService` exposes a static `/site/` without a bearer header over localhost and the Tailscale IP. In tests it returned HTTP 200 while `/api/status` returned 401 without the bearer token. Site files MUST NOT contain secrets. A phone preview is **not** a private data server until site auth is added or the inbound port removed.
- A temporary `phone-agent` helper on Oracle holds a short-lived app API session key. This is a migration aid and not part of the target. Do not commit/reveal it; remove only **after** direct-agent validation or on explicit Stop/Unpair.
- The imported Android project is not a GitHub-released or release-signed app; the observed APK is a Gradle debug build. Never claim it is production hardened.
- The imported APK includes **no** screen/touch automation, root shell, Shizuku, inbound ADB bridge, or native Infra gateway enrollment.
- The v0.1 `specialUse` foreground-service justification has not undergone app-store review; private sideload behavior and Play distribution must be assessed separately.

## Permission plan (default deny)

| Data/ability | OS mechanism | Approval and revocation |
| --- | --- | --- |
| Local app workspace | app-private files | Agent foreground service manual Start/Stop |
| Chosen documents | SAF `ACTION_OPEN_DOCUMENT_TREE` / persisted URI grants | On-phone folder picker; each grant removable |
| Battery/system metadata | normal Android APIs | visible connected session; no device fingerprint data |
| Notifications | NotificationListenerService, **later only** | separate system setting, scoped read controls, off by default |
| App interaction | AccessibilityService, **later only** | separate system consent, session indicator and immediate Stop |
| Screen frames | MediaProjection, **later only** | per-session user consent Android 14+; session must be visible |
| Audio/camera/location | Android runtime grants, not in MVP | require fresh explicit feature consent, scope and indicators |
| Dev environment | dedicated Termux IPC/app sandbox, later | explicit install/setup, independently disableable |
| System/root | unsupported | do not route ADB via privilege escalation or exploit |

Avoid ``MANAGE_EXTERNAL_STORAGE``, SMS/Call Log permissions, device-admin role, auto-install/unknown sources privileges, notification-listener or Accessibility privileges merely for connectivity.

## Transport and policy checks

- Only outbound `wss://` to configured gateway; TLS verification required and no hardcoded secrets.
- No unconditional trust-all certs; certificate pinning only if rollover/repair planned.
- API transport is **Infra agent wire protocol**, never the owner OAuth credential.
- Connection queue and workers bounded; avoid holding wakelocks continually for mere presence.
- Use manual Start and user-visible foreground notification. Android may kill a process; reconnection and offline state must be honest.
- Deliberately preserve working desktop v1 and the live Node v0.3 rollback path.
- Existing Rust gateway already supports revocation and connection-generation supersession; tests must confirm Android follows these semantics.
- For Google Play release, check Accessibility/foreground-service policy and distribution disclosures; do not assume sideload success implies store approval.

## Acceptance suite

### Build and interop
- [ ] Android Gradle assemble/debug and repeatable CI from clean SDK.
- [ ] Rust workspace `cargo fmt`, `cargo test`, and relevant gateway auth tests unaffected.
- [ ] Android unit tests verify exact Rust signed-auth bytes and serde wire JSON.
- [ ] No private credentials/signing seeds in tracked files, build outputs or logger.
- [ ] Static manifest review for overly broad permissions and exported components.

### Pair/auth/security
- [ ] Fresh one-time code -> public key enrolled -> Android appears under existing `devices` MCP tool.
- [ ] Same code second device rejected; reused/expired code rejected.
- [ ] Ed25519 invalid signature, wrong nonce, bad epoch, wrong instance ID rejected.
- [ ] Stolen/replayed handshake cannot impersonate later socket.
- [ ] Unpaired/revoked device cannot reconnect; UI reports clear state.
- [ ] Connection from untrusted gateway fails TLS verification.
- [ ] Maximum frame, wrong protocol, unknown operation handled without crash.

### Functional behavior (no ADB helper after initial install)
- [ ] From ChatGPT, use `devices` on Android and ordinary `system summary` without referring to Oracle `exec`.
- [ ] Read battery, OS, storage and network from authorized APIs.
- [ ] App workspace list/read/write and transfer work through existing `files`/`transfer` tool families.
- [ ] SAF denied until user picks folder; revoking grant removes access immediately.
- [ ] Traversal, symlink, sibling-document access and large-file denial tests pass.
- [ ] All unsupported `exec`/`containers`/`services`/`terminal` operations return explicit unsupported.
- [ ] New Android device never claims root, ADB shell or data from other apps.

### Lifecycle/reliability
- [ ] Screen on and off, at least one hour of idle, charging/unplugged and battery saver conditions.
- [ ] Wi-Fi -> mobile -> Wi-Fi; VPN/Tailscale disconnect/reconnect; no duplicate device entries.
- [ ] Gateway restart/reconnect; stale sockets retired by connection generation; no replay of writes.
- [ ] Android process death: device goes offline; reconnect only when OS permits.
- [ ] Reboot: no misleading always-online claim; user starts app again unless opt-in boot policy is validated.
- [ ] Service Stop/force-stop, permission revoked, Unpair and server revocation update gateway presence.
- [ ] Test without Tailscale and without USB after enrollment (gateway WSS reachability permitting).
- [ ] Monitor memory/battery/network consumption under idle, 10 requests, large transfer, connection flapping.

### Release/rollback
- [ ] Side-by-side test gateway or isolated registry; no unreviewed change to existing deployed gateway.
- [ ] Install and restore current v0.1 safely; no forced loss of selected SAF grants without UX.
- [ ] Only after green gates: turn off inbound 8765, remove temporary Oracle helper key, confirm phone still connected as signed agent.
- [ ] Pin source commit, SDK/JDK, APK signing scheme, SHA-256 and controlled distribution. APK release signature must differ from throwaway debug build.
- [ ] Capture redacted acceptance evidence: tests, connection status, denial paths; no secrets or screenshots of personal data.

## High-risk design decisions to resolve before implementing

1. **Outbound-only WSS endpoint:** choose an already authenticated, trustworthy private TLS route to `/agent`. Branded public dashboard might intentionally reject that endpoint. No new public inbound port as a shortcut.
2. **Android Ed25519 key storage:** portable software Ed25519 with hardware-backed AES wrapping if available, or proven device-native equivalent; test real Motorola Keystore support.
3. **How far generic files reach:** app sandbox and SAF are intentionally narrower than desktop user filesystem. Choose honest `unsupported` rather than loosening Android permissions.
4. **FGS subtype:** verify long-running user-initiated agent session fits `specialUse` and distribution expectations. No mislabeling as media, health, remote messaging or VPN.
5. **Termux trust:** if implementing later, approve the bridge and command scopes separately. An unverified third-party executable/plugin cannot get general shell rights by virtue of gateway pairing.
6. **Session approval:** screen/camera/messaging or destructive file writes need positive opt-in independent of continuous agent presence.

## Official documentation (reviewed 2026-10-18)

- https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start
- https://developer.android.com/develop/background-work/services/fgs/service-types
- https://developer.android.com/develop/background-work/services/fgs/changes
- https://developer.android.com/topic/performance/background-optimization.html
- https://developer.android.com/training/monitoring-device-state/doze-standby
- https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec
- https://developer.android.com/training/data-storage/shared/documents-files
- https://developer.android.com/about/versions/14/behavior-changes-14
- https://developer.android.com/guide/topics/ui/accessibility/service
- https://developer.android.com/privacy-and-security/local-network-permission
