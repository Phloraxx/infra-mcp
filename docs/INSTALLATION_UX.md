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

## Backend progress — 2026-10-15

The enrollment backend now implements the identity flow required by this UX: one-time expiring code, locally generated Ed25519 keypair, idempotent retry for the same enrollment attempt, persisted stable device ID, signed challenge authentication, and revocation. The development agent can enroll from `INFRA_ENROLLMENT_CODE` and persist its identity under `INFRA_STATE_DIR`.

This is not yet the user-facing installer. Native packaging, startup registration, protected OS credential storage, repair/doctor, permission onboarding, and dashboard-driven Add Device remain required before the installation UX is considered complete.
