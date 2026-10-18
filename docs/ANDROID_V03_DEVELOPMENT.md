# Infra Phone v0.3 — SenseGuide development companion

Status: implemented as a separate debug package `com.phloraxx.infraphone.dev` on `feat/android-agent-plan`; stable Infra Phone and stable SenseGuide remain distinct. Signed Infra gateway target: `https://infra.drbijoy.in/`.

## Implemented

1. **Signed binary transfer** through the existing Rust Infra v1 `transfer` RPC (no inbound HTTP port, no ADB in normal use). Upload/download begin/chunk/commit/status/list/cancel supported for workspace-relative files only. Each chunk is at most 128 KiB, file maximum 256 MiB, SHA-256 checked on upload commit and download completion, retries of already written chunks must byte-match. At most eight transfer handles; handles persist across WebSocket reconnects while the foreground service process lives. Transfer process restart aborts incomplete handles; restart upload from scratch. Android OS private workspace containment inherited from `ScopedWorkspace`. **Never upload a secret/private dataset to a public repo.**
2. **User-approved SenseGuide test-APK installation**. Upload with signed transfer to `apks/senseguide-debug.apk`. Open Infra Phone Dev > **Open developer controls** > **Review staged SenseGuide APK**. The app validates the staged APK archive has package `io.github.phloraxx.senseguide.lab.bridge`, displays it and requires a local confirmation. Android's own installer requires further owner action and may request the standard Install Unknown Apps permission. No remote or silent install. Stable SenseGuide `io.github.phloraxx.senseguide.lab` is never an allowed target via this control.
3. **Restricted development command interface** over signed `exec`: exact allowlist `help`, `senseguide status`, `senseguide workspace`, `senseguide request-launch`, `senseguide request-install`. The last two merely post a local notification leading to the owner approval screen; if notifications are disabled, the owner can open DevTools manually. No commands passed to `sh`, `bash`, `su`, ADB or Termux, and no arbitrary working directories. This **is not a full Android terminal**. Full development shells belong on the connected Mac/Oracle; Termux integration would require separate explicit setup and access controls.
4. **Optional restricted screen inspection and tap** through Android's Accessibility service, which the owner must explicitly enable under Android Settings. Owner additionally enables a 5-minute session inside Infra Phone Dev. Lease expires based on elapsed and wall clock and boot identity; requires the Infra signed agent to be active. Only the foreground SenseGuide Lab/Diagnostics window can be interacted with; denies other apps including installer, system settings, banking, chats. Limited `desktop.permissions`, `desktop.screenshot` (JPEG downscaled and capped), `desktop.click` (single tap), `desktop.key` (`back` only). Does not allow arbitrary text entry, window scraping, clipboard reading or background screen capture. Screenshot displays actual phone foreground view and may contain sensitive information. Do not record other people without consent. Not a stealth screen service.
5. **Background reliability**. The already user-initiated gateway foreground service requests `START_STICKY` recovery if killed, retains heartbeat and bounded reconnect. Optional **Reconnect signed gateway after phone reboot** preference registers a `BOOT_COMPLETED` receiver. Android battery optimization, foreground-service launch restrictions, reboot/lock, force-stop and revoked permission may still prevent automated reconnect. No root/bootloader unlock or bypass. Service notification has Disconnect action.

## Using these operations from ChatGPT / MCP

`devices` should show Android as v0.3.0 with capabilities `status`, `system`, `files`, `transfer`, `exec`, `desktop`. For binary transfer through the Infra plugin:
- `transfer.upload_begin(path="apks/senseguide-debug.apk", size=..., sha256=<expected hash>, create_parents=true, overwrite=true)` yields `transfer.id` and `chunk_size`.
- `transfer.upload_chunk(transfer_id=..., offset=..., data_base64=...)` repeated in order, offsets from `next_offset`.
- `transfer.upload_commit(transfer_id=...)` verifies digest before atomic placement. **This only stages the APK**.
- `exec(command="senseguide status")` returns installed package versions and staged APK size.
- `exec(command="senseguide request-install")` asks the owner via notification; **only** the local developer controls can proceed to the standard Android installation UI.
- `desktop(action="permissions")` identifies whether a foreground SenseGuide + Accessibility + 5-min on-phone consent allows temporary screenshot/taps.
- Other requested ops return explicit unsupported/refused errors.

## Safety/acceptance gates

- [x] Android JVM tests + debug build succeed with v0.3 source.
- [ ] Debug APK installed in place on Motorola preserving Ed25519 enrollment.
- [ ] Signed MCP Android capabilities and `exec help` and `senseguide status` pass.
- [ ] Verified binary upload/download SHA-256 roundtrip and retry/negative-case checks on phone.
- [ ] APK install UI and denial tested; **do not silently install** SenseGuide.
- [ ] Android owner manually enables Accessibility service and five-minute session, then scoped `desktop.screenshot/click` verified against foreground SenseGuide.
- [ ] Screen-off/Doze/Wi-Fi↔mobile data, device reboot, gateway reboot and explicit disconnect reconnection tested.

The APK is a developer preview. Do not claim autonomous obstacle safety, generic root/ADB access, unattended app installs, a full OS terminal, or unrestricted screen control.
