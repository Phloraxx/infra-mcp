# Infra v0.1.1-rc.4 Release Candidate

This is an **early test release**, not a recommended production installation.

Infra MCP is a private, self-hosted control plane: one Rust gateway you run, signed native agents on macOS, Windows and Linux that dial out to it, an embedded dark dashboard, and a compact MCP tool surface whose terminals, jobs and transfers survive disconnects.

## In this candidate

- Native Infra CLI and agent for Windows, macOS, and Linux (x86_64 and ARM64), with owner-approved device-initiated pairing, per-user startup registration, and infra status / doctor / repair recovery commands.
- Persistent PTY sessions, durable background jobs, resumable SHA-256-verified transfers, and structured system, network, service, container and git operations.
- Signed Ed25519 release manifest and SHA-256 asset digests for the built-in updater; the manifest signature and release checksums are distributed with all binaries.
- Embedded dependency-free dashboard with owner sign-in (Argon2id), live state over server-sent events, browser terminal and bounded file browsing.
- The Android companion (Infra Phone) lives in apps/android-agent and is developed in this repository; it is not part of the six desktop assets below.

## Changed since v0.1.1-rc.3

- Dependency refresh: rusqlite 0.41.1, ignore 0.4.34, getrandom 0.4.3 and GitHub Actions bumps, plus the RustCrypto migration to sha2 0.11.1, hmac 0.13.0 and argon2 0.6.0.
- Android: consolidated Infra Phone UX, in-app self-update and wireless ADB research (pull request #41).
- Tests: the protocol and local-state crates now pin their full wire contract, including operation envelope tags, result shapes, authentication framing and version-gated state files; workspace tests grew from 129 to 145.
- Docs: the five dashboard documents were consolidated into one canonical dashboard document, the README was rewritten around the shipped control plane, and these release notes were refreshed.
- Gateway healthz reports the exact version and commit identity (introduced in rc.3).

## Important limitations

- **First-install scripts are not independently publisher authenticated.** The release manifest authenticates the native updater, but the bootstrap PowerShell/POSIX scripts currently validate download integrity using SHA-256 sidecars fetched from the same GitHub release. For this release candidate, obtain binaries only from this repository's official release page and verify their signed release manifest with the separately trusted publisher public key (see the [release publisher key](https://github.com/Phloraxx/infra-mcp/blob/main/docs/RELEASE_PUBLISHER_KEY.md)). Do not pipe unreviewed scripts from arbitrary endpoints.
- **No automatic fleet update:** publishing this release does not replace installed device agents. Pairing still requires owner approval. The gateway/dashboard rollout is performed separately with backups and rollback. No consumer-ready single-click installer is included.
- Physical Windows login/reboot, restricted-user policy, automatic upgrade/rollback and non-VPN public WSS ingress still need field validation. Do not migrate existing healthy agents automatically.
- These are pre-release developer/testing binaries without platform notarization or code-signing of each executable. Your OS may display untrusted publisher warnings. Do not disable security settings to bypass them.

Source and development history: https://github.com/Phloraxx/infra-mcp/compare/v0.1.1-rc.3...v0.1.1-rc.4
