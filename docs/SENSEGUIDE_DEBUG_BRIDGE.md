# SenseGuide developer diagnostic intake — Infra Phone v0.2

**Scope:** Cross-app, user-approved metadata import for SenseGuide experimental debug builds.
**Implemented on:** `feat/android-agent-plan` (Infra-MCP) + `feat/infra-diagnostics-bridge` (SenseGuide).

## Contract

SenseGuide `Assist` collects at most 100 allowlisted in-memory events only after tapping **Start debug recording**. It never exports camera images, audio, recognized words, GPS or private chat messages. SenseGuide requires **Pause assistance** before the export button becomes available.

1. SenseGuide creates up to 64 KiB JSONL with first line `{"schema":"senseguide.debug.v1",...}`.
2. It sends explicit action `io.github.phloraxx.infraphone.IMPORT_DIAGNOSTICS` to `com.phloraxx.infraphone.dev/com.phloraxx.infraphone.ImportDiagnosticsActivity`, with URI `content://io.github.phloraxx.senseguide.lab.bridge.debugreport/report` and temporary Android `FLAG_GRANT_READ_URI_PERMISSION` (plus ClipData grant). SenseGuide provider is nonexported and rejects any path besides `report`.
3. Infra Phone displays a separate on-phone **Import report** confirmation. The receiver accepts only `content://` from allowlisted SenseGuide authorities, requires the temporary read grant, enforces 64 KiB max and schema prefix, and writes atomically into its private workspace at `senseguide/reports/diagnostics-<timestamp>.jsonl`.
4. The already enrolled and online Android agent exposes the report via its existing signed `files list/read/hash` through `https://infra.drbijoy.in/`. **No extra Infra-MCP endpoint, public HTTP server, privileged app permission or new token** was added.

## Privacy, scope and failure handling

- This is manual **one-shot** report transfer, not continuous telemetry or remote video. No Android Accessibility, MediaProjection, READ_LOGS, MANAGE_EXTERNAL_STORAGE or root required.
- Receiver is exported solely for explicit import intents and does **not** silently persist input. The user can cancel. Only URI grants for precisely one report are valid; it cannot read SenseGuide's app-private data generally.
- All report names are generated locally by Infra Phone; sender cannot set path. Input bounded; schema checked; no executable code, shell commands, or model weights accepted as installation actions.
- Imported JSONL is within Infra Phone's existing `files` workspace. It remains there until deliberately removed; avoid sending or committing contents to GitHub, and keep reported camera/depth events synthetic versus observed clearly distinguished.
- Report metadata and 100-event ring can reveal assistive activity patterns even without imagery; request on-phone approval and exercise retention hygiene.
- Stored report is not a safety proof. Missing/low-confidence depth still means unavailable, never a safe-to-walk signal.
- Do not replace the stable SenseGuide package or remove Infra Phone Dev's enrolled app data during installation. Use matching Android signing key and `adb install -r` for development only, or user-approved APK install later.

## Acceptance checklist

- [x] Infra Phone receiver compiles under Gradle `:app:testDebugUnitTest :app:assembleDebug` with existing signed WSS agent unchanged.
- [x] SenseGuide journal tests and debug build compile with SDK 36; no changes to warning decision formulas.
- [ ] Install both update packages; keep stable SenseGuide and enrolled Infra Phone data intact.
- [ ] In SenseGuide Diagnostics tap Start debug recording, wait for a safe stationary sample, Pause assistance, Send.
- [ ] Confirm Infra Phone import screen requires approval and can read the granted URI, then verify report under `senseguide/reports/` using direct Android MCP `files`.
- [ ] Verify no permission means no import, wrong authority refused, active assistance cannot export, and sizes >64 KiB rejected.
- [ ] Test user-facing imported file deletion/retention controls in a later milestone.

## Engineering next

Develop and verify signed binary `transfer`, controlled install approvals via Android PackageInstaller, and an optional on-device diagnostic replay mechanism. Do **not** elevate to generic shell/ADB or covert screen capture. For SenseGuide sensing limits and research scope see `Phloraxx/SenseGuide/docs/11_STATUS.md`.
