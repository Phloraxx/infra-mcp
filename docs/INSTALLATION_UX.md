# Installation and onboarding UX

## Desired first-run flow

From the dashboard:

1. Click **Add device**.
2. Choose macOS, Windows, or Linux.
3. Give the device an optional friendly name; default to the hostname.
4. The dashboard creates a short-lived enrollment request.
5. Show one primary installation action for the selected OS.
6. Installer performs preflight checks, downloads/verifies the native binary, installs startup integration, generates a device keypair, enrolls the device, starts the agent, and verifies connectivity.
7. The dashboard transitions the pending card to **Online** without requiring a refresh.
8. Show detected capabilities and any optional permissions still needed, such as macOS Accessibility or Screen Recording.

The normal path should not require manually editing files, creating service units, copying secrets, or restarting a terminal.

## Installation surfaces

### macOS / Linux quick install

A one-line bootstrap command is acceptable as the fastest path, but it must only bootstrap a signed native installer/binary. The bootstrap script should be small, auditable, retry downloads, verify signatures/checksums, and leave useful diagnostics on failure.

Long term, provide a signed `.pkg` for macOS and native packages where useful.

### Windows

Provide a signed installer executable/MSI as the primary path. A PowerShell bootstrap may remain as an advanced/automation option.

### Headless Linux

Support a non-interactive install with explicit flags for servers and CI-like environments.

## Enrollment design

- Enrollment ID/code is short-lived and single-purpose.
- Enrollment is idempotent for the same installation attempt so network retries do not create duplicate devices.
- The device generates its asymmetric identity locally.
- The gateway stores only the public identity and metadata needed to authenticate the device.
- A completed enrollment cannot be replayed for a second device.
- If an enrollment expires during download, the installer should provide a simple re-enrollment path without requiring reinstall.

## Installer state machine

Use explicit phases and display them:

1. Preflight
2. Download
3. Verify
4. Install
5. Register startup
6. Enroll
7. Start
8. Connectivity check
9. Permissions check
10. Complete

Failures should name the failed phase and provide a machine-readable diagnostic code.

## Network resilience

The bootstrap/downloader should:

- retry transient TLS/network failures with backoff
- support resume where practical
- try supported alternate download methods when the platform's default utility is unavailable
- never execute a partially downloaded binary
- distinguish DNS/TLS/proxy/authentication failures

## Startup behavior

### Desktop user mode

Normal desktop installations start automatically when the user logs in and continuously reconnect thereafter.

### Server mode

Linux servers can install a system service that starts at boot.

### Full-control mode

Where privileged operations are enabled, install a separate privileged helper/service. The user-session agent remains responsible for interactive desktop/PTY operations that require the signed-in session.

## Uninstall and repair

The same product should own its lifecycle:

- `infra status`
- `infra doctor`
- `infra repair`
- `infra reconnect`
- `infra update`
- `infra uninstall`

`doctor` should diagnose service registration, binary integrity, credential storage, network reachability, DNS/TLS, gateway authentication, permissions, version skew, duplicate instances, and local port/IPC conflicts.

`repair` should safely fix common deterministic problems and clearly state what it changed.

## Acceptance requirements

A fresh supported machine should reach an Online state through the normal install path without reading documentation. Re-running the installer on an existing healthy installation must be safe and should repair/upgrade rather than create a second logical device.

## Implemented product foundation — 2026-10-15

The repository now implements the first complete onboarding path rather than only the enrollment backend:

- Dashboard **Add device** creates a short-lived one-time enrollment and generates the recommended macOS/Linux or Windows bootstrap command.
- `scripts/install.sh` and `scripts/install.ps1` install two native binaries: `infra` and `infra-agent`. They require no Node/npm/Python runtime on the target machine.
- Release mode downloads platform/architecture-specific GitHub Release assets and verifies SHA-256 sidecars before replacement. `--source-dir` / `-SourceDir` exists only for isolated development smoke tests. Cryptographically signed release/update metadata is still a required hardening step.
- `infra install` writes versioned local config, performs one-time enrollment through `infra-agent --enroll-only`, and then registers automatic startup unless explicitly disabled. The enrollment code is not written to config.
- The agent subsequently reads its gateway/device name from local config and authenticates with the persisted Ed25519 identity.
- Startup adapters exist for systemd-user on Linux, LaunchAgent on macOS, and an ONLOGON Scheduled Task on Windows.
- `infra status`, `doctor`, `repair`, `reconnect`, and `uninstall` operate on the same local state/startup model. `doctor` currently covers config, identity, runtime PID, startup state and gateway health.
- Re-running `infra install` with an existing identity does not require or consume a new enrollment code; it can re-register startup around the same logical device.
- The isolated installer/dashboard smoke test proves the configure/enroll/reconnect/status/doctor path without registering a real startup service.

Still required before the UX is considered production-ready: real-device startup/reboot acceptance tests on all primary OSes, better phase-specific installer progress/error codes, native package installers where useful, OS protected credential stores/ACL hardening, `infra update` with staged rollback, permission onboarding, and production dashboard authentication.
