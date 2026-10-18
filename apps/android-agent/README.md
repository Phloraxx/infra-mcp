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
