# Android / Infra Phone — current status

> **Later context (2026-10-23):** This page is the **2026-10-18 v0.3.0 acceptance snapshot**, not proof of the latest installed APK. A newer v0.3.1 Android source/build is on the Mac, with one-hour SenseGuide consent and an owner-reviewed Infra Phone self-update workflow; **on-phone installation and end-to-end verification are not established here**. Read [2026-10-23 handoff](ANDROID_HANDOFF_2026-10-23.md) for source reconciliation, and [embedded ADB research/plan](ANDROID_ADB_RESEARCH_AND_PLAN.md) before attempting privileges. On-device ADB is **not implemented** in the documented build.

**Verified on:** 18 October 2026 (IST)
**Application:** Infra Phone Dev `com.phloraxx.infraphone.dev` v0.3.0, Android 16, Motorola Edge 60 Stylus
**Branch:** `feat/android-agent-plan`
**Connection:** outbound authenticated WebSocket to `https://infra.drbijoy.in/`; normal signed RPC does not require USB, wireless debugging, or Tailscale
**Safety scope:** user-owned device, no root, no bootloader unlock, no unrestricted Android shell.

> This page is the **current acceptance baseline**. Older v0.1/v0.2 plan sections elsewhere in the repository are retained as dated implementation history. Do not read their unchecked boxes or historical "not implemented" statements as current product status.

## Verified on the real phone

| Feature | Evidence | State |
| --- | --- | --- |
| Signed agent registration/reconnect | Native MCP `devices` showed the phone online as Android agent v0.3.0 with `status/system/files/transfer/exec/desktop`. Enrollment preserved through app upgrade. | Verified |
| Private-workspace files | Signed list/read/hash/edit operations; imported SenseGuide diagnostic JSONL retrieved. | Verified |
| Binary upload/download | Deterministic 132,100-byte two-chunk round-trip, repeated-chunk idempotency, matching SHA-256, zero changed bytes and traversal denial. | Verified |
| Real SenseGuide APK staging | Full 12,451,391-byte Diagnostics APK transferred via the Mac and signed MCP to `apks/senseguide-debug.apk`, with independent matching SHA-256. | Verified |
| Restricted `exec` | `senseguide status`, `request-launch`, `request-install` and rejection of unsupported commands/working directories. | Verified |
| User-approved APK update | Android package metadata shows SenseGuide Diagnostics 0.5.5 retained its package and was updated on 18 October; owner reviewed installation. Stable SenseGuide Lab 0.5.5 preserved. | Verified at package level |
| Accessibility consent | Owner enabled Android Accessibility and a separate five-minute consent session; native `desktop.permissions` reported active consent and foreground SenseGuide. | Verified |
| Screen inspection/tap | Signed `desktop.screenshot` returned JPEG 431 x 960; `desktop.click` returned success on a non-interactive SenseGuide title. | Verified |
| Cross-app diagnostics | Owner exported bounded metadata-only JSONL with explicit import approval; signed MCP read the recorded depth/warning/haptic events. | Verified |
| Android and CI builds | v0.3 Gradle JVM tests and debug APK build succeeded; Android GitHub Actions passed. | Verified |

## Available capabilities

- `status`: agent status.
- `system`: Android device summary.
- `files`: app-private workspace only; selected supported operations (not unrestricted filesystem access).
- `transfer`: chunked binary staging within private workspace, capped at 128 KiB per chunk and 256 MiB per file, SHA-256 verified. Incomplete transfer handles need restarting if the service process dies.
- `exec`: strictly allowlisted SenseGuide commands; **not** a general terminal, `adb`, `sh`, or Termux bridge. In the MCP connector use `cwd=""`; nonempty working directories are refused.
- `desktop`: Android owner-enabled Accessibility plus a separate five-minute foreground SenseGuide-only consent. Supports permission probe, downsampled screenshots, taps, and Back; other applications and generic clipboard/typing are excluded.

## Owner-facing workflow

1. Open **Infra Phone Dev** and press **Connect signed agent** if its session is offline. The separate **Start agent** button is for the legacy local HTTP helper and is not required for the signed connection.
2. Compile SenseGuide Diagnostics on the trusted Mac, verify APK SHA-256, use signed `transfer` to stage it at `apks/senseguide-debug.apk`, and run `senseguide status` to confirm staged size.
3. On the phone, open **Open developer controls → Review staged SenseGuide APK**. Review and confirm the package; Android requires its own installation authorization. **No remote silent installation.**
4. To inspect the UI, manually enable **Infra Phone: SenseGuide-only controls** in Accessibility settings, then enable a **five-minute session** in Infra Phone Dev. Open SenseGuide Diagnostics in the foreground. Revocation/expiry or leaving SenseGuide should prevent screen control.
5. For actual sensing diagnostics, tap **Start debug recording** inside SenseGuide Diagnostics, perform a stationary lab test, **Pause assistance**, **Send debug report to Infra Phone**, and approve **Import report**. Retrieve only metadata JSONL in `senseguide/reports/` through signed MCP.

No camera frames, recorded voice transcripts, or user-location history are part of the diagnostic export. Screenshot access can reveal what the camera UI displays while explicitly authorized; use only controlled scenes.

## Not yet established

- Long-term reliability under screen-off/Doze/battery optimization, Wi-Fi↔mobile-data handoff (including without VPN), gateway restart, reboot/login, and service process death.
- Negative screen-control tests after expiry/revocation and when an unrelated application is in the foreground **during** a still-valid consent lease.
- Automatic, owner-approved APK install + launch + repeatable testing without manual Android prompts. Installed app update was verified at the package level, not via an end-to-end APK signature/installer telemetry audit.
- Full system terminal, unattended ADB functions, privileged OS-wide app control, SAF-backed arbitrary folders, or general screen/typing automation.
- Actual SenseGuide obstruction-detection accuracy, continuous fresh ARCore depth, and field safety; 31 seconds of metadata showed usable depth intermittently, multiple unavailable transitions, and haptic requests but cannot prove physical detection correctness.

## Next actions

1. Run the explicitly safe reliability/negative-permission test matrix on the owner-unlocked phone; stop and request owner action when Android requires it.
2. Improve SenseGuide's test harness for depth freshness, accepted/rejected measurements, haptic causality and repeatable stationary scenarios. Preserve all fail-closed warning rules; physical navigation trials need supervision.
3. Consider SAF and optional Termux integration as separate, consent-scoped milestones; do not assume root or silent installation.

## Source-of-truth links

- [Android v0.3 design, protocol and live test evidence](ANDROID_V03_DEVELOPMENT.md)
- [SenseGuide diagnostic import contract](SENSEGUIDE_DEBUG_BRIDGE.md)
- [Implementation status (chronological)](IMPLEMENTATION_STATUS.md)
- [Roadmap (historical plus current)](ROADMAP.md)
- [Android app and legacy helper notes](../apps/android-agent/README.md)

**Credential hygiene:** Never write enrollment codes, device signing seeds, bearer tokens, real private screenshots, recorded diagnostics, or APK artifacts to git.
