# One-command Infra setup

**2026-10-24 · Proposed, not implemented or deployed.**

This is the new near-term onboarding priority, refining [ONBOARDING_REDESIGN.md](ONBOARDING_REDESIGN.md). A signed MSI can be offered later, but is no longer the prerequisite for a usable desktop installer.

## Owner experience

**Copy setup command → Infra detects this computer → Approve pairing in browser → Connected.**

No required device name, gateway URL, enrollment code, port, VPN, manually chosen architecture or runtime dependency. The native agent reads the operating-system hostname. A later dashboard Rename action sets an alias without changing identity.

For Windows and Unix, use **two tiny bootstrap scripts** (PowerShell and POSIX sh) feeding **one shared native Rust setup implementation**. The scripts are only download/verification adapters, never independent lifecycle managers. The existing Rust CLI gains a simple setup operation that checks existing state, reuses signed identity, performs device-initiated pairing through the existing gateway, installs native per-user startup, and verifies an authenticated online response. Re-running it repairs problems without creating a duplicate device.

The dashboard hides command complexity behind **Copy Windows command**, **Copy macOS/Linux command**, and an optional **Download installer** action. The standard script must not use a pre-issued code that expires while software is downloading; owner approval happens after the device generates a local verification code. Explicit browser owner confirmation and a key-bound challenge remain required.

## Implementation contract

1. **Release/trust:** Current GitHub releases are empty. Publish signed, versioned binaries for supported operating systems before promising that the public installer URL works. An ordinary SHA-256 checksum served from the same host is an integrity check, **not publisher authentication**. Script bootstrap trust must be explicit: TLS on a trusted canonical domain, authenticated release metadata and a pinned signer where possible; a signed native distribution is the stronger first-install trust root.
2. **Preflight:** detect platform/architecture, live canonical host, network/TLS/proxy and per-user filesystem permissions before modifying an existing install. Missing releases, 404s and HTML bodies must produce clear failures, not an empty script or incomplete files.
3. **Atomic install:** stage and verify CLI and agent as one version, preserve old version and device identity, then safely activate. Roll back on failed readiness. Windows running binaries need deferred replacement, not unsafe overwrite.
4. **Pairing:** local CLI generates/preserves a pending signing key and short comparison code; authenticated owner approves in the existing gateway dashboard. Approval alone isn't proof of connection. Expiry/rejection can be retried without reinstallation.
5. **Native startup:** install, start and **check actual execution**, including Task Scheduler on Windows, launchd on macOS, systemd user on Linux. Do not ignore task launch errors or announce success after mere registration.
6. **Ready probe:** require fresh signed agent connection plus status through the gateway; otherwise show which phase needs action. Differentiate Installed, Pairing, Connecting, Connected, and Needs Repair.
7. **Repair/upgrade:** re-running setup detects an already enrolled machine, preserves one stable ID, fixes only failed steps and does not silently revoke/re-enroll. Keep explicit local diagnostics and an advanced headless mode.
8. **Visibility:** human error + recoverable next action for DNS, TLS, proxy, unsupported OS, release unavailable, verification failure, pairing expiry, startup denied, offline agent, locked files and rollback.

## Windows-specific rule

Windows PowerShell 5.1 needs explicit safe web parsing after the December 2025 security update. Never rely on a download piped blindly to a script evaluator as the *dependable* default. Download first, validate the result and authenticated source, then run the local bootstrap. Do not disable Defender, SmartScreen or execution-policy safeguards. Offer a verified native binary installer when scripts are blocked.

## Acceptance / sequence

**Order:** published test release → shared Rust setup/repair state machine → key-bound device-initiated pairing → thin verified PowerShell/POSIX bootstrap → streamlined dashboard → isolated CI and physical Windows/Mac tests → optional MSI/PKG later.

Test Windows PowerShell 5.1 and 7, Windows x64/ARM64, paths with spaces, standard-user permissions, no release, bad DNS, downloads truncated, expired pairing, repeated setup, startup denied, reboot/reconnect, identity preservation, repair and rollback. The Mac/Linux paths must meet the same observable contract with native startup adapters. Do not cut over the current working Oracle/Mac/Android agents or gateway until staging and rollback are proven.

**Not required:** Electron, Tauri, Node runtime, a new daemon or backend, a privileged service, a separate broker, custom UDP transport or a second installer logic tree.
