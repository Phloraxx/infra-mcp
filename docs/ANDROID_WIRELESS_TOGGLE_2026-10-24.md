# Android Wireless Debugging Toggle — 2026-10-24

**Status:** debug-only experiment in v0.3.4-lab (versionCode 7), stacked on the local ADB pairing probe. **Not deployed or owner-granted by this change.**

## Decision

Infra Phone can control **standard Android 11+ Wireless debugging** without Shizuku by using `Settings.Global.putInt(..., "adb_wifi_enabled", 1/0)`, **after** a one-time shell grant of `android.permission.WRITE_SECURE_SETTINGS`. Android's ADB settings observer handles starting/stopping adbd. This is separate from legacy `adb tcpip 5555` and does not authorize a new ADB host key.

**This permission is broad**, so the manifest declaration exists only in `src/debug/AndroidManifest.xml`, and calls exist only in a debug-only Activity and helper. Nothing in the signed Infra RPC dispatcher invokes it. The owner must explicitly tap each toggle. No background auto-on, auto-off, remote shell, Shizuku clone, or always-on adbd is added in this phase.

## One-time setup

After installing the debug-signed APK **over** the previous `com.phloraxx.infraphone.dev` (preserving its enrollment), on a previously authorized ADB computer run:

```sh
adb shell pm grant com.phloraxx.infraphone.dev android.permission.WRITE_SECURE_SETTINGS
```

The app has a **Copy one-time grant command** button; a Mac command may optionally add `-s stylus:5555` before `shell` if multiple devices are connected. Do not publish/share the one-time grant command with shell pairing keys or use it on untrusted machines.

In Infra Phone → **Local ADB pairing lab**, refresh status, tap **Enable Wireless debugging** when needed, and **Disable Wireless debugging** when done. Android can prompt to trust the network, refuse enabling without Wi-Fi, or behave differently on an OEM build. The toggle reads the actual stored system setting, not the availability of a TLS listener. **A signed, paired TLS client remains a separate requirement for ADB shell access**. The first pairing requires the owner-provided six-digit code.

## Platform distinctions and security

- `adb_wifi_enabled`: standard Wireless debugging with authenticated TLS pairing and a dynamically allocated connection port. Android's `AdbSettingsObserver` watches this setting. Existing pairing keys may survive restart. This is our **preferred** mode.
- `service.adb.tcp.port=5555`: legacy network ADB mode started via `adb tcpip 5555` (requires an existing privileged ADB transport), generally bound to a network interface, and **not controlled by `adb_wifi_enabled`**. Do not silently enable or automatically maintain port 5555. On this Motorola, this was observed while `adb_wifi_enabled=0` and is the Mac's existing development connection.
- Granting `WRITE_SECURE_SETTINGS` does not create shell UID privileges, authorize our own new ADB host key, or safely grant general remote shell. It **only** permits this app to write privileged settings, with all of that permission's attendant security risks. Separate local owner session gating remains mandatory before exposing elevated RPC.
- Disabling Wireless debugging can disconnect active authorized clients. Always ask in the on-phone UI. The implementation does **not** disable USB debugging or Developer options, and does **not** promise to close legacy 5555.
- Standard Wireless debugging may be unavailable on cellular data or in an untrusted Wi-Fi environment. Do not suggest that a setting of `1` guarantees a listener is reachable.
- **Potential auto-on later:** when a separately authorized one-hour elevated session is requested and an approved pairing exists, enable standard Wireless debugging, connect via loopback, perform narrowly scoped operations, then turn it back off **only if Infra turned it on** and the owner has no conflicting session. Do not implement silent, unconditional background elevation or automatic external ADB-port exposure.

## Verification checklist

1. `testDebugUnitTest` and `assembleDebug` pass with R8. Manifest has `WRITE_SECURE_SETTINGS` only for debug variants; minified production variant does not declare it.
2. Verify same package signing certificate, proper increasing versionCode (v0.3.3 recovery code6; lab code7), and enrollment preservation during upgrade.
3. Without permission, UI displays the copyable grant and enable/disable fails closed.
4. After authorized grant, status reports permission and toggles the setting. On Android 16 Motorola, confirm real TLS adbd port is advertised when Wi-Fi is connected, and that Android trust prompts are respected.
5. Check that disabling one mode doesn't silently terminate the owner's still-needed legacy 5555 session. Don't force-enable developer settings or disable USB debugging.
6. Pair using Android's one-time six-digit code; verify `uid=2000(shell)` using the loopback-only fixed `shell:id` probe, no remote shell.
7. Test Wi-Fi loss, reboot, incorrect pairing code, process restart, sign-in/out, revocation, and when an existing trusted debugger is connected.
8. Revoke the broad setting permission if no longer wanted: `adb shell pm revoke com.phloraxx.infraphone.dev android.permission.WRITE_SECURE_SETTINGS`. Do not uninstall the app just to revoke.

## Research / sources

- AOSP adbd lifecycle: https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/main/docs/dev/how_adbd_starts.md
- Android protected permission: https://developer.android.com/reference/android/Manifest.permission#WRITE_SECURE_SETTINGS
- DevSwitch, working Android 16 example with one-time grant (GPL-3.0; **reference only, not copied**): https://github.com/retransmit/DevSwitch
- Shizuku pairing and reboot caveats: https://shizuku.rikka.app/guide/setup/
- Prior plan: [Embedded ADB research](ANDROID_ADB_RESEARCH_AND_PLAN.md) and [local probe](ANDROID_ADB_LOCAL_SPIKE_2026-10-23.md).
