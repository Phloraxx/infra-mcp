# Infra Phone — Android companion

**Current status: imported v0.1 prototype (October 18, 2026).** The buildable Android APK is installed on a Motorola Edge 60 Stylus, but **this source does not yet connect as a signed Infra gateway agent**.

This module is the baseline for the Android first-class-device work. Read [Android integration plan](../../docs/ANDROID_AGENT_PLAN.md), [wire protocol adapter](../../docs/ANDROID_PROTOCOL_ADAPTER.md), and [security/acceptance gates](../../docs/ANDROID_SECURITY_AND_TESTS.md) before modifying it.

## What is implemented

- Native Java Android app; minSdk 28, targetSdk 35, compileSdk 35.
- User taps **Start agent** and **Stop agent**; visible foreground-service notification.
- Session-unique bearer token shown in the app.
- HTTP server limited to `127.0.0.1:8765` and available IPv4 Tailscale tunnel addresses (no Wi-Fi/mobile listener).
- `GET /api/status`: app/device/battery/storage metadata.
- `GET /api/files`, `GET /api/file`, `PUT /api/file`: workspace-only scoped files with size/path checks.
- `GET /api/shared` and `GET /api/shared/file?name=...`: **one user-selected SAF folder**, subject to persisted Android permission.
- `GET /site/`: simple local/Tailscale static web preview.

**Security caveat:** `/site/` does **not** require bearer authorization in v0.1. All peers able to reach its tailnet listener can read hosted static files. Use only nonsensitive test assets and do not publish secrets. `/api/*` endpoints require a bearer key, and the current helper on Oracle only accesses this v0.1 API. No backend public port should be opened.

This APK has **no** native gateway device registration, outbound WebSocket connection, root shell, Android screen control, Shizuku integration, remote OS-level install, or general reading of other apps' private data. It should not be described as an ADB replacement.

## Build

From this directory (requires Java 21 and Android SDK platform/build tools):

```bash
./gradlew :app:assembleDebug
```

Debug APK: `app/build/outputs/apk/debug/app-debug.apk`. Install with `adb install -r ...` if USB is available. The same app can be sideloaded manually after building; normal operation of the v0.1 HTTP API does not need USB once the app is installed and paired with a client.

The initial build was validated on a Mac with Gradle 9.6.1 / JDK 21 and installed on Android 16. **Building the imported sources from this git checkout in CI or from a clean SDK remains to be rechecked.** Do not commit `local.properties`, signing keystores, .env files, credentials, or generated APKs.

## Current v0.1 local test

1. On phone: open Infra Phone, **Start agent**.
2. Copy the displayed session key privately.
3. On trusted USB-connected computer (test only): `adb forward tcp:8765 tcp:8765`.
4. `curl -H "Authorization: Bearer <TOKEN>" http://127.0.0.1:8765/api/status`.
5. Optional upload to private workspace:

```bash
curl -X PUT -H "Authorization: Bearer <TOKEN>" --data-binary @index.html \
  "http://127.0.0.1:8765/api/file?path=site/index.html"
```

6. Stop the agent in the phone app or notification to invalidate the session key.

## Target architecture (NOT YET IMPLEMENTED)

```text
ChatGPT → existing Infra v1 Rust MCP gateway ← outbound signed WSS ← Android app
```

New functions should use the **existing Rust Wire Protocol v1**, the existing gateway `/enroll`/`/agent` routes and capability-aware tool dispatch. The final direct agent should not require the temporary Oracle `phone-agent` helper or inbound port `8765`.

Read `AGENTS.md` in this folder for integration conventions.


## v0.2 signed direct-agent development (2026-10-18)

The Android module now contains a second **development** mode, alongside the v0.1 local HTTP fallback. Its debug application ID is `com.phloraxx.infraphone.dev`, so it can be installed alongside the stable v0.1 package.

Source implemented:
- Per-installation Ed25519 signing seed encrypted under AndroidKeyStore AES-GCM.
- Existing gateway one-time `POST /enroll` flow through a user-visible pairing form.
- Outbound signed WebSocket client for existing Rust `/agent`, challenge/hello/ack, heartbeat, reconnect and stop.
- Minimal `status` and `system summary` requests; all other operations explicitly unsupported.
- Dev-only cleartext allowance restricted in the app's URL validator to loopback/Tailscale IPv4 addresses, for an isolated lab gateway. Production uses verified HTTPS/WSS.
- Rust + Android JVM SHA-256 golden fixture for the exact signed-auth bytes.

**Verification:** Gradle `:app:testDebugUnitTest :app:assembleDebug` passed on Mac; Rust `cargo test -p infra-protocol` passed; v0.2 dev package installed and launched on Android 16 without runtime exceptions. An isolated Rust gateway and SSH/ADB test tunnel were reachable. The secure `/api/enrollments` route rejected requests lacking its bearer credential as expected. **Real phone enrollment and device visibility through MCP have not been verified**; the one-time-code UI automation was not completed. Do not report this as end-to-end complete.

A development user must generate a one-time code from an owner-authorized gateway, enter its URL and code in the dev app, press **Enroll phone**, then **Connect signed agent**. The current debug-only loopback lab address is not a production configuration. For real operation use a valid certificate on an accessible WSS gateway. Do not embed owner credentials in the APK.

The existing v0.1 HTTP service remains as an explicit fallback until direct-agent acceptance. The new signed agent **does not** gain root, ADB, screen access, or arbitrary app data access. Files/transfer, production gateway configuration and end-to-end validation remain next milestones.

## v0.2 signed-agent workspace file subset (2026-10-18)

The outbound signed gateway agent now dispatches the existing Infra `files` operation for **only the app-private workspace**, rooted at `getFilesDir()/workspace`. Supported file actions: `list`, `stat`, `read` (byte-offset UTF-8), `hash` (SHA-256), `mkdir`, `write` (staged atomic replace), and `exact_edit` (expected match count).

Send relative paths such as `site/index.html`; use `""` to list the workspace root. Absolute Android paths, `..` escapes, symlink escapes, NUL, and backslashes are rejected by `ScopedWorkspace`. Writes/inline reads/edits are capped at 1 MiB, and directory listings at 500 entries. Large binary transfers are **not implemented yet** in the outbound signed protocol. `append`, `delete_permanent`, `trash`, `copy`, and `move` explicitly return `unsupported` in this milestone. Never claim the phone has arbitrary Android filesystem access.

File operations run in a bounded Android worker pool; unsuccessful authorized operations return a structured `failed` result rather than closing the signed connection. New `ScopedWorkspaceTest` exercises traversal and symlink denial in JVM tests. Gradle `:app:testDebugUnitTest :app:assembleDebug` passed on Mac after this change.

**End-to-end caveat:** The automated entry of the one-time enrollment credential did not complete, so these operations are build-tested but **not yet validated through a live phone-to-MCP session**. Existing v0.1 HTTP fallback remains separate.
