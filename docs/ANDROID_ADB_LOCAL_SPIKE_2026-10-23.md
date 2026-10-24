# Local Android ADB probe (debug-only) — 2026-10-23

Status: **code experiment, not device-verified and not an Infra MCP feature**. This is P1 from [the embedded ADB plan](ANDROID_ADB_RESEARCH_AND_PLAN.md). It is deliberately separate from the signed agent request dispatcher and from any `exec` or desktop permissions.

## Verified device baseline

The authorized Oracle ADB client reached the Motorola Edge 60 Stylus running Android 16 and obtained `uid=2000(shell)`. At inspection, the installed Infra Phone Dev package was still **v0.3.0**, Shizuku was installed, Android's Wireless debugging setting was off, and a **legacy** ADB TCP listener was available on all interfaces at port 5555. A process on the phone could connect to its own loopback port 5555; **this is not an encrypted/pairing-mode acceptance test**. Avoid exposing that listener beyond the user's trusted network and retire it once secure Wireless debugging is established.

The v0.3.1 APK was built on the Mac, integrity-verified through Infra transfer to Oracle, and a replacement install was attempted. Android did not report a confirmed install; subsequent package metadata remained v0.3.0. A second diagnostic installer call timed out with unknown outcome and was not blindly repeated. Do not claim v0.3.1 deployed. A copy of that APK was staged temporarily under `/data/local/tmp/` for diagnosis; clean it only after verifying installer processes/sessions are finished.

## What this branch adds

- Debug-only `AdbProbeActivity`, declared **non-exported** in `src/debug/AndroidManifest.xml`; visible from the main screen only in debug builds.
- Debug-only `LocalAdbClient` using the externally maintained Apache-2.0/GPL-3.0-dual-licensed `libadb-android` 3.1.1, with an **independent** 2048-bit RSA key in Android Keystore. No ADB key reuse from Infra Ed25519 gateway enrollment.
- Pair with user-entered Android Wireless debugging pairing port and six-digit code, against **127.0.0.1 only**. Code is cleared from the UI immediately; no code/keys/command output transmitted over Infra MCP.
- Connect to user-entered Wireless debugging **connection** port (different from pairing port); run only fixed `shell:id` and verify `uid=2000(shell)`. No other remote shell execution is added.
- Explicit disconnect and owner-visible status. Legacy port **5555 is rejected** even though Oracle uses it for device-side development. App works without this experimental feature.
- The third-party dependency and JitPack resolver are scoped to the debug build and narrow repository groups. No direct Shizuku copy, extra remote endpoint, or privileged background service.

## Security constraints / blockers before anything beyond local P1

1. `libadb-android` is not independently security audited; its current `SslUtils` trusts all server certificates and caches an `SSLContext` statically. Loopback-only and fixed `shell:id` limit this experiment's impact, but a production privileged action path must address server identity and key-handling risks **before** enabling remote execution.
2. The library's dependency graph includes BouncyCastle and SPAKE2; we exclude its older BouncyCastle module in favor of the application's current `bcprov-jdk18on` version. Verify correct pairing/test behavior, JNI dependencies, transitive licensing and debug APK size in CI; don't infer size savings from fewer installed applications.
3. No background service or automatically reconnecting ADB client is added. No automatic elevation or persistently authorized remote shell. There is no one-hour **ADB** lease yet; the existing one-hour consent (in the v0.3.1 source) is **SenseGuide-only screen control**.
4. The debug probe must not be treated as production-ready: operations currently run in a worker thread without a fully proven cancellation/timeout path, and device-specific pairing/keystore/TLS behavior is not yet validated. These are mandatory P1/P2 gates.
5. For the user flow on one phone, Android's pairing dialog can lose focus when switching apps. Use split-screen or the system pairing-code notification as needed; avoid accessibility-based or hidden permission tricks.

## Acceptance procedure (owner present)

1. Build signed debug APK in CI or on the trusted Mac. Inspect package name/signature, APK-size delta and dependencies.
2. **Preserve current Infra enrollment**; update installed `com.phloraxx.infraphone.dev` with an APK signed by the same key and a newer versionCode. **Never uninstall** to work around a signing mismatch. Debug branch versionCode must be incremented before installation.
3. On Android open Developer options → Wireless debugging, enable it manually, choose **Pair device with pairing code**.
4. In Infra Phone debug app → **Local ADB pairing lab** enter pairing port/code shown by Android. Then enter the separate connection port shown on the main Wireless debugging screen.
5. Verify a successful local read-only `uid=2000(shell)` result; confirm that no `exec` MCP request was needed.
6. Test wrong/expired code, port change, Wireless debugging off, reboot, app stop/force-stop, user disconnection, device encryption/locked screen, and when an unrelated app is foreground. Confirm Infra's normal signed capabilities still work without ADB.
7. Compare debug APK/resident RAM/battery against P0; audit transitive dependencies. Only then consider a *separately approved* one-hour elevated RPC scope.

**No-go:** no remote shell, silent install, direct adbd port 5555, Shizuku/Binder duplication, root, or privileged persistence in the first embedded-ADB prototype.

## Primary references

- https://developer.android.com/tools/adb
- https://developer.android.com/studio/run/device
- https://github.com/MuntashirAkon/libadb-android
- https://github.com/tytydraco/ladb
- https://developer.android.com/privacy-and-security/keystore
- https://developer.android.com/privacy-and-security/local-network-permission

## 2026-10-24: APK footprint experiment (build-verified, phone not yet tested)

A non-debuggable, R8-minified **development** APK on the same dev application ID `com.phloraxx.infraphone.dev` and versionCode 5 built successfully using the Mac's existing debug signing identity. It retains the debug source-set pairing UI, but because Android Gradle sets `BuildConfig.DEBUG=false` for a non-debuggable build, the local-only lab gating explicitly checks `BuildConfig.BUILD_TYPE == "debug"` and package suffix `.dev`.

Measured build APKs:
- Unminified P1 development APK: 9,982,785 bytes.
- R8-minified development APK: approximately 1.79 MB (verify current exact bytes before deployment).
- Android manifest still lists non-exported `AdbProbeActivity`, preserved `MainActivity` and outbound gateway service.
- Retains the SPAKE2 JNI class name with a targeted R8 keep rule. BouncyCastle's unused algorithms are stripped rather than including the full provider.
- `:app:testDebugUnitTest :app:assembleDebug` succeeded locally; signer fingerprint matched the installed debug APK's previously verified Mac key.

**Tradeoffs and release gates:** the development build is now non-debuggable (`run-as`/Java debugger unavailable). Its previous debug-only cleartext enrollment fallback is disabled by `BuildConfig.DEBUG`, but existing signed HTTPS/WSS enrollment should work. Runtime Keystore, Ed25519 signed reconnection, SPAKE2 native pairing and APK installation have **not** yet been verified with the minified binary on the actual Motorola. Do not treat an R8 build pass as runtime proof; preserve the existing installed v0.3.1 until a rollback path and owner approval are ready.

## 2026-10-24 live continuation — verification and remaining blocker

- **Real phone:** Infra Phone Dev v0.3.1 remains online over the normal signed outbound WSS session, same enrolled device identity; no new experimental APK has been installed. Local SenseGuide Accessibility is enabled but the owner's one-hour screen-control consent is currently inactive.
- **Android package signatures:** v0.3.2 minified ADB experiment, v0.3.3 recovery APK, and the installed v0.3.1 use the Mac development signing identity (SHA-256 `96700e7a318f73ae2ea9167330ae29da03fcb760cb6798a340b918112f8258fc`); Android version codes are 5, 6, and 4, respectively. Verify signer and installed package again before any later install.
- **Recovery APK on Mac:** `~/Downloads/InfraPhone-v0.3.3-recovery-debug.apk` (9,722,603 bytes; SHA-256 `e05e26aafaef54abef832d5f47a96fb5e3eff2507f743588ca089fbb4feb147a`). It is built from the pre-ADB v0.3.1 functionality with an increased versionCode to allow an *in-place* recovery from experimental code 5; **do not uninstall** an enrolled app to roll back.
- **Embedded ADB slim APK on Mac:** `~/Development/infra-android-adb-probe-20261024/apps/android-agent/app/build/outputs/apk/debug/app-debug.apk`, version 0.3.2 code 5, minified/non-debuggable dev flavor, 1,823,167 bytes, SHA-256 `c3d9df02003f3fb229c968b0c9d89a07491fb6e3a7505ca20f7f7a236b8aafa7`. Mac `testDebugUnitTest`, `assembleDebug`, `apksigner verify` and manifest inspection passed; **no physical-device execution or SPAKE2 pairing acceptance**.
- **Negative on-phone self-update test passed:** writing a deliberate 1,856-byte non-APK to the update staging path yielded `ready:false` / `APK is not for this Infra Phone installation`. Clearing the file to zero bytes caused `infra request-update` to fail with `Transfer an Infra Phone APK ... first`. No notification or installation requested. The update path now contains a zero-byte placeholder; delete it with an authorized app-workspace cleanup when supported. This does **not** establish full signature verification or positive installer flow.
- **Positive transfer blocked:** signed `transfer` staging of the previous 9,722,579-byte v0.3.1 APK reached 5,373,952 bytes, then later transfer calls were repeatedly rejected. Both partial transfer sessions were canceled; no upload commit or installation occurred. Do not infer success or blindly repeat an unknown-outcome package operation.
- **Oracle ADB unavailable now:** the former `stylus:5555` listener is unreachable; same phone remains online through Infra's non-ADB WSS. Don't reopen legacy all-interfaces TCP ADB merely to bypass pairing or transfer problems. Test the new paired-TLS mode with Android's own Wireless debugging switch when the owner enables it.
- **Potential client timeout risk:** upstream `libadb-android` pairing uses a potentially blocking socket/TLS handshake, and the local lab reader waits for `shell:id`. Current P1 has no proven end-to-end cancellation for a peer that stalls; before P2/MCP elevation, add bounded cancellation and investigate the library's permissive TLS trust and static SSL context. Keep generic remote shell disabled.

**Next validated milestone:** owner-approved delivery of the smaller signed APK (version 5) through a supported path, verify Infra reconnect and local pairing UI. If runtime fails, update in place to the separately built versionCode 6 recovery APK, retaining the same package/signing identity. Then, with Android Wireless debugging enabled and its pairing code displayed by the owner, verify only `uid=2000(shell)` locally and test revocation. Do **not** advertise ADB as available through MCP before this completes.
