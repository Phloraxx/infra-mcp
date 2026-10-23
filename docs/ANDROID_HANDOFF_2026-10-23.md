# Infra Phone handoff / source reconciliation — 2026-10-23

**Audience:** next human or coding agent working in \`Phloraxx/infra-mcp\`. **Goal:** make current Android source reproducible before adding ADB. Do not assume the GitHub default branch is the latest Android build.

## Where the code actually lives

| Location | Known state | What to do |
| --- | --- | --- |
| GitHub \`main\` | Older Infra v1 design scaffold; Android files/docs absent in default-branch content lookups | Do **not** use as Android source until feature branch merges |
| GitHub \`feat/android-agent-plan\` | Android v0.3.0 dev APK source, signed WSS + files/transfers, SenseGuide-only screen/installer flows, Android acceptance documentation | **Canonical version-controlled Android starting point** |
| Oracle \`/home/drvij/infra-mcp-v1-work\` | Working copy on \`feat/android-agent-plan\`; at inspection had two local commits ahead plus uncommitted unrelated gateway/OAuth changes and backup files | **Do not** reset/rebase/commit/push unrelated work; use isolated worktree or GitHub branch |
| Mac \`~/Development/InfraPhoneNext\` | Unversioned Android Gradle source, newer than the checked GitHub branch; v0.3.1 debug APK built on 2026-10-23 | **Source to review/reconcile**, not an authoritative Git checkout |
| Motorola Edge 60 Stylus | Android 16/API 36; signed Infra Android agent was observed online as v0.3.0 at last verified device check | Recheck **installed** package version after owner completes update; build success does not equal deployment |

## Mac-only changes to reconcile, source files

All relative to \`app/src/main/java/com/phloraxx/infraphone/\` on the newer Mac source.

- \`ScreenConsent.java\`: replaces hard-coded 300,000 ms with a single 3,600,000 ms constant; still checks monotonic/wall clock, boot count and agent activity.
- \`DevToolsActivity.java\`: one-hour UI labels + explicit Infra Phone self-update review button and Android installer flow.
- \`SenseGuideAccessibility.java\`: one-hour consent label/error, still limited to foreground SenseGuide screenshot/tap/back; not a generic phone UI driver.
- \`InfraSelfUpdate.java\`: new helper validates staged \`apks/infra-phone-update.apk\` package equals this app, matches current APK signing certificates, blocks version-code downgrades, posts on-device notification; returns standard Android package installation intent.
- \`StagedApkProvider.java\`: provider access for validated Infra Phone self-update alongside existing SenseGuide Diagnostics staged APK.
- \`SenseGuideDevelopment.java\`: adds the exact commands \`infra update status\` and \`infra request-update\`; these **request review** and do not install silently.
- \`GatewayAgentService.java\`: defaults missing \`cwd\` to empty for allowlisted exec; advertises 0.3.1; no ADB support.
- \`MainActivity.java\`: developer-controls copy updated.
- \`app/build.gradle\`: version code 4, version name 0.3.1; \`com.phloraxx.infraphone.dev\` is the debug application ID.
- \`README.md\`: appended description of on-device self-update and rollout.

The previous Mac build and newer v0.3.1 debug APK were reported as signed by the **same Mac debug signing certificate** (certificate SHA-256 fingerprint should be verified again during reconciliation). This does **not** prove the installed device's signature or production keystore matches.

**Build evidence:** \`./gradlew :app:testDebugUnitTest :app:assembleDebug --offline\` succeeded on Mac with Java 21/Android SDK; staged APK metadata showed \`com.phloraxx.infraphone.dev\`, versionCode=4, versionName=0.3.1. APK is stored only on the Mac under Downloads; **do not commit any APK, signing key, enrollment data, screenshots, phone files or tokens.** v0.3.1 self-update has **not** passed an on-phone end-to-end test in this handoff.

## Clean reconciliation recipe

1. Prepare a **new clean worktree** based on the latest intended Android feature branch; preserve Oracle's working tree and live services.
2. Copy only inspected *source and docs* from Mac. For the Android module compare directory trees with \`diff -ru --exclude=build --exclude=.gradle --exclude=local.properties --exclude=.idea\`, or use \`git diff --no-index\`. Review package IDs, Android manifest, signing behavior, provider read path, consent defaults, API wire shapes and \`cwd\` forwarding.
3. Apply incremental commits: (a) consent; (b) self-update; (c) corrected exec/version/docs. Do not import generated Gradle state or local secrets. Existing feature branch has historical v0.1/v0.2 notes: label them as history, don't erase.
4. Rebuild in clean checkout; check test summary, manifest, APK package/versionCode, signing fingerprint; run Rust protocol compatibility tests if dispatch semantics change.
5. Have the owner manually install or approve update and verify preserved enrollment, presence, \`infra update status\`, staged signature validation, Android install prompt and one-hour consent expiry/revocation. A successful \`assembleDebug\` is **not** device acceptance.
6. Only after parity: begin [embedded ADB spike](ANDROID_ADB_RESEARCH_AND_PLAN.md). Keep first spike debug-only, read-only, and gated.

## Safe research/release boundaries

- The existing Infra Rust gateway serves **all** devices. Do not redeploy/restart it, modify Cloudflare/OAuth/Tailscale rules, or replace the live Node fallback solely for Android experimentation.
- Standard Android agent + SenseGuide-only Accessibility continue functioning without developer options, Wireless debugging, Shizuku, or root.
- Existing \`exec\` is a tiny allowlist. Changing this to general shell is a **privilege change**, not routine polishing.
- One-hour consent is **SenseGuide screen control** in the Mac source; a future ADB privilege lease needs a **separate** enable/revoke state.
- ADB pairing state may outlive a lease, but remote elevated command authority must not.
- Record actual installed package metadata and permission state after each release; don't infer from APK path, signed connection, or source version string.

## What is verified vs open

**Verified:** GitHub repo/feature branch layout; existing v0.3.0 Android documented acceptance; Mac v0.3.1 build/tests and APK package/signature measurement; ADB installed on Mac with no connected device at last inspection.

**Open until tested:** whether owner has installed v0.3.1, end-to-end Infra Phone self-update, direct same-phone ADB TLS pairing/reconnect on Motorola Android 16, Android local-network/mDNS behavior, library transitive dependencies/size, 60-minute lease expiry on physical device, and any ADB-related API approval UX.

## Next coding task

**First:** reconcile v0.3.1 into \`apps/android-agent\` in a clean isolated checkout and prove build/phone parity. **Second:** implement only P1 direct loopback ADB pairing/read-only \`uid=2000(shell)\` experiment (see [plan](ANDROID_ADB_RESEARCH_AND_PLAN.md)). Do not implement full Shizuku/Binder or an unrestricted remote shell before P1 evidence.
