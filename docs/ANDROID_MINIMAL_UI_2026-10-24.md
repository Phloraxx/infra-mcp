# Infra Phone: minimal UI redesign — 2026-10-24

**Status:** v0.3.8-ui-lab debug build, versionCode 9, designed as a safe in-place update over the enrolled v0.3.4-lab (code 7). This is a user-facing hierarchy and presentation change, **not** an MCP protocol redesign or new remote permission model.

## Product intent

Infra Phone should feel like a quiet, trustworthy companion—not a debug console. The normal user should only need to see whether their phone is connected and have a single primary action. All other controls belong in deliberately discoverable secondary screens.

### Screen hierarchy

1. **Home:** Minimal brand header, recognizable phone name, signed Infra connection state, one context-sensitive connect/disconnect action, gateway domain, and one Android-access shortcut (debug build). Unpaired installations see an equally simple one-time pairing screen; successful enrollment immediately attempts the signed connection.
2. **Settings:** Rarely changed preferences and grouped tools: Android access, SenseGuide, an expandable legacy local API, opt-in reboot reconnection, and a clearly confirmed Forget action. Do **not** display local API tokens by default; reveal them only through an explicit Copy action.
3. **SenseGuide:** One contextual screen-access button (enable for 1h or revoke), one diagnostics action, Android Accessibility settings link, and update actions **only when an APK is staged**. Keep all installation and screen consent prompts on-device, and preserve notification-triggered approvals.
4. **Android access (debug only):** The already-running localhost TCP 5555 listener is offered as the single direct Connect option. The app never starts or exposes that port. The insecure legacy transport is clearly disclosed. Secure wireless TLS debugging and its one-time pairing controls are collapsed behind an alternative section. No commands routed to remote MCP; identity probe is only fixed `shell:id`.

## Visual language

Charcoal canvas, slate surfaces, muted sage typography, a single desaturated mint accent; custom Android vector glyphs rather than emoji or stock photography. Generous vertical rhythm, medium-weight sans serif, simple one-column cards, subtler borders and no large marketing copy. Android standard screen readers/press feedback remain supported. All UI implemented without a Material/Compose library migration or another external dependency.

## Preserved behavior / invariants

- Enrolled `DeviceIdentity` and signing key never reset by an app update.
- Gateway agent still starts only on explicit owner action or existing opt-in reboot rule; disconnect remains available.
- Local `AgentService` remains independent from signed Infra gateway, with its private session key only copied on demand.
- Folder permissions use Android `ACTION_OPEN_DOCUMENT_TREE` with persistable read/write grants.
- One-hour SenseGuide-only consent, Accessibility restriction, manual revocation, on-device owner prompts preserved.
- Signed/staged self-update path and SenseGuide APK installation retain package validation and the system installation dialog.
- `WRITE_SECURE_SETTINGS` is still an explicit debug-only one-time grant; turning standard Wireless debugging on/off remains locally user-operated.
- Local ADB `shell:id` probe is not a general or remotely callable shell. Legacy TCP 5555 is never enabled by the app.
- No gateway, Rust, Oracle, systemd, Tailscale, or production Android package changes.

## Acceptance and rollback

1. Build `:app:testDebugUnitTest :app:assembleDebug` in a clean checkout, verify R8, manifest exported activities, package name/version/signing certificate and APK footprint. Preserve a same-signer recovery APK with a **higher versionCode 10** before phone installation.
2. Install code9 in place, confirm old Android package UID, preserved signing enrollment and successful WSS reconnection.
3. Open **Home, Settings, SenseGuide, Android access** on the actual Motorola. Check UI layout at physical resolution via `uiautomator dump` or screenshots *only with Infra Phone foreground*, not other private apps. No clipping, overlapping or unnecessary settings on Home.
4. Confirm Connect/Disconnect, reboot preference, staged update flow, optional local API, one-hour consent and debug pairing screens remain functional; run negative/permission tests without changing unrequested Android developer settings.
5. If runtime or UI regressions remain, **install** the existing-codebase, same-signer `v0.4.0-recovery` APK (code10) over the test, without uninstalling and losing enrollment. Existing key/identity should survive.

## Recovery build (Mac)

`~/Downloads/InfraPhone-v0.4.0-recovery-debug.apk`, versionCode10, SHA-256 `92fc2bea929875af45318f0e5cb638bf02ac58d9ce4c58ac82460b5a6a2ad549`. Built from the previously working app, with no UI redesign. This is an emergency fallback APK, not the desired release.

## Follow-up improvements (not implemented here)

- An opt-in, Android-compliant resume after package update (`ACTION_MY_PACKAGE_REPLACED`) to avoid manual reconnection, without silently overriding Disconnect.
- Run production-ready local ADB TLS pairing checks and harden the unaudited library before general remote privileges.
- Real one-hour elevated Android session requires separate owner permission, expiration and revocation; the existing screen-control lease must not be reused implicitly.
