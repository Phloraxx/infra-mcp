# Platform implementation notes

Infra presents consistent MCP semantics while using native platform behavior underneath.

## Shared requirements

Every platform implementation should expose the same high-level capability model but may report unsupported actions explicitly.

Primary build targets:

- Linux ARM64
- macOS ARM64
- Windows x64

Secondary later targets:

- Linux x64
- Windows ARM64

## Linux

### Startup

- user install: systemd user service where available
- optional system install/helper: systemd system service

Support non-systemd/manual mode later only if it provides real value.

### Terminal/process

- Unix PTY
- process groups for tree cancellation
- `/proc` when useful, with portable fallbacks where practical

### Files/search

Normal POSIX semantics. Be explicit about symlinks, mount boundaries, permissions, and trash support.

### Desktop

Later phase:

- AT-SPI accessibility where a desktop session exposes it
- compositor/session-specific screenshot/input fallbacks as needed

Headless server support must never depend on desktop libraries.

### Privileged helper

Unix domain socket with strong filesystem permissions plus peer-credential checks where available.

## macOS

### Startup

Use launchd. User agent should survive normal logins/restarts and reconnect after sleep/wake.

### Terminal

Unix PTY with correct zsh/bash environment behavior. Avoid assuming interactive shell startup files are loaded for background launchd processes.

### Credentials

Long-lived device signing keys are now stored through Keychain Services when available. `identity.json` retains only versioned metadata/key ID in native-store mode; legacy file identities migrate automatically after credential read-back verification.

### Desktop

Later phase:

- Accessibility API for semantic UI tree/actions
- native screen-capture APIs
- native input events
- permission detection and clear `doctor` guidance for Accessibility/Screen Recording permission

A small Swift/Objective-C shim is acceptable where Rust bindings are fragile. Keep the public agent protocol in Rust.

### Privileged helper

Use an Apple-supported service/helper strategy rather than running the entire agent as root.

## Windows

### Startup

User mode may use Scheduled Task or an equivalent robust login startup mechanism. Full/system mode may install a Windows service for the helper.

### Terminal

Use ConPTY for interactive terminal sessions. Process-tree cancellation must account for Windows job objects/process groups rather than only killing the immediate child.

### Credentials

Long-lived device signing keys are now stored through Windows Credential Manager. `identity.json` retains only versioned metadata/key ID in native-store mode. Broader Windows state-directory ACL hardening remains pending.

### Files

Normalize behavior carefully for:

- drive-letter paths
- UNC paths if supported
- case-insensitivity
- file locking
- long paths
- recycle bin vs permanent delete

### Desktop

Later phase:

- Windows UI Automation
- native screen/window capture
- SendInput or appropriate controlled input API

### Privileged helper

Windows service communicating over authenticated/restricted named pipe or equivalent local IPC.

## Capability reporting

Agent hello should advertise capability flags derived from runtime checks, not merely platform name.

Examples:

- `pty`
- `docker`
- `git`
- `desktop.capture`
- `desktop.accessibility`
- `clipboard`
- `privileged_helper`

This lets the gateway fail cleanly before dispatching unsupported actions.


## Android (new planning target — not yet an Infra agent)

The signed Android-first-class-device plan is in [ANDROID_AGENT_PLAN.md](ANDROID_AGENT_PLAN.md); wire interoperability is in [ANDROID_PROTOCOL_ADAPTER.md](ANDROID_PROTOCOL_ADAPTER.md). The imported v0.1 Java APK source is at `apps/android-agent/`.

- Android 16/API 36 was observed on the first target device, but do not hardcode device-specific identity. The prototype currently targets API 35 with minSdk 28.
- Use existing `DeviceHello.platform = "android"`, `arch = "aarch64"` (from runtime), per-device Ed25519 enrollment and verified outbound `wss://.../agent`. No separate Android MCP server.
- Use Android foreground service and visible Stop UI; Android may suspend/kick out long-lived network tasks. No promise of background-start exemptions, always-on or boot persistence.
- Do not equate Android app UID with ADB shell/Linux user. Generic `exec`, `pty`, root, system files, containers and OS service tools are unsupported by default.
- Scoped files are app workspace plus SAF user-chosen trees; expose the appropriate `files`/`transfer` subsets with explicit unsupported and permission-denied results. No blanket storage access.
- Protect Ed25519 identity at rest with AndroidKeyStore-wrapped encryption where native Ed25519 signing keys aren't portable.
- Optional later screen control needs Accessibility and MediaProjection with separate on-phone consent. No permission escalation while pairing.
- Disconnect/outcome-unknown semantics, capability reporting, revocation and gateway auth must match the Rust protocol.
