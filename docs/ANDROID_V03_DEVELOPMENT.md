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

## Live v0.3 end-to-end acceptance — 2026-10-18

**Observed and verified using normal Infra-MCP connector operations; no ADB used in the signed transfer path:**

- Registered Motorola appeared online with `agent_version=0.3.0`, generation 17, and capabilities `status`, `system`, `files`, `transfer`, `exec`, `desktop`.
- `exec(command="senseguide status", cwd="")` succeeded and identified both stable SenseGuide Lab and separate SenseGuide Diagnostics 0.5.5 packages. The connector's omitted/default working directory is currently rejected; explicitly set `cwd=""` for the safe command allowlist. An unrestricted `pwd` was correctly rejected.
- `desktop.permissions` reported Accessibility disabled, five-minute consent disabled, and foreground SenseGuide false; **no screenshot/click permission was granted or exercised**.
- A disposable 132,100-byte binary was uploaded in two chunks, with SHA-256 predeclared. Retrying the initial 131,072-byte chunk was accepted idempotently without advancing the offset; upload commit returned the expected SHA-256 `48a20eae42bbc77a9c24d4284d45d83668640645568a3894ad24853339bc3e33`. Two download chunks returned all 132,100 bytes exactly, with **zero byte mismatches** compared to deterministic source bytes.
- Path traversal via `transfer.upload_begin("../outside-private-workspace.bin")` was rejected with `Path escapes app workspace`.
- A real **12,451,391-byte** SenseGuide Diagnostics APK from the connected Mac was relayed using Mac `transfer.download_chunk` → Android `transfer.upload_chunk` in <=128 KiB chunks, and committed successfully at `apks/senseguide-debug.apk`. The destination's independent `files.hash` matched the Mac origin: `a55769552de9081d77b0ed85cdcdf8ae9b7855a94aceb65b670d880c0b9bcc0d`.
- The Android `senseguide status` operation confirmed `staged_apk_bytes=12451391`. The `senseguide request-install` restricted command was accepted and returned the owner-review instruction; **it did not install or launch the APK**. The owner still must explicitly approve the APK review and the Android system installer. Do not infer approval or installation completion.
- Completed transfer handles were cancelled/cleared; cancelling a completed transfer does not delete the committed destination file. The synthetic smoke-test file may remain in `infra-smoke/` and can be deleted during later scoped cleanup.

**Remaining acceptance:** APK review screen, package match and system installer confirmation on-phone; Android Accessibility service enabled by owner and five-minute SenseGuide-only screen session; screen-off/Doze and network handoff and reboot recovery. No change to SenseGuide's actual camera/depth/voice/haptic logic. A request with default nonempty `cwd` and bounded transfer-handle cleanup ergonomics are implementation polish.

## 2026-10-18 — Live owner-enabled APK/Accessibility acceptance

After owner completed the phone-side setup, Infra `devices` confirmed Android v0.3.0 **online** at connection generation 17, advertising `status`, `system`, `files`, `transfer`, `exec`, `desktop`.
- Signed `exec("senseguide status", cwd="")` succeeded. Stable `io.github.phloraxx.senseguide.lab` and separate `io.github.phloraxx.senseguide.lab.bridge` both still report version 0.5.5. The staged Diagnostics APK is 12,451,391 bytes. Omitting `cwd` with the current MCP wrapper may send an unsupported default; passing `cwd=""` fixes this and the correct error path was observed for other values.
- Android package metadata confirmed Diagnostics latest `lastUpdateTime=2026-10-18 23:17:59`, `firstInstallTime=2026-10-18 20:44:40` (device-local), establishing that the separate developer package was updated on-device without uninstalling it.
- `desktop.permissions` confirmed Accessibility service enabled and separate five-minute on-device screen-control consent active. Initially `foreground_senseguide=false`, correctly distinguishing the foreground-app gate.
- Requested `senseguide request-launch` over signed `exec`; a local approval notification was posted, without bypassing the owner. After foreground changed to SenseGuide Diagnostics, `desktop.permissions` confirmed `foreground_senseguide=true`.
- Signed `desktop.screenshot` returned a valid `image/jpeg` image (431 × 960, downsized) and signed `desktop.click(x=600,y=170,button="left")` returned `ok=true` on a static/noninteractive SenseGuide title, rather than a test-warning button. No images were persisted or committed; this test does **not** establish navigation safety.
- `transfer.list` and workspace `files.list` succeeded. The previously verified 132,100-byte synthetic file roundtrip and 12,451,391-byte signed APK stage are documented in the prior acceptance checkpoint. No new install or file mutation was requested during these UI tests.
- **Not yet checked**: denial outside SenseGuide with an actually active five-minute lease and untrusted app in foreground; expiry at five minutes, screen-off/Doze/battery kill, Wi-Fi/mobile transition, fresh reboot and foreground-service relaunch. Do not claim the user-enabled Accessibility service is appropriate outside a controlled testing session. Manually disable it when not needed.

No generic shell, ADB-equivalent privileged control, background recording, silent APK install or unapproved application tapping was enabled.
