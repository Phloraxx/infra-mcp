# Infra v0.1.0 Release Candidate — native desktop agents

This is an **early test release**, not a recommended production installation.

- Native Infra CLI and agent for Windows, macOS, and Linux (x86_64 and ARM64).
- Owner-approved, device-initiated pairing (no manually issued code), auto-detected device hostname, optional separate display alias, and recoverable setup.
- Signed Ed25519 release manifest and SHA-256 asset digests for the built-in updater; manifest signature and release checksums are distributed with all binaries.
- Windows per-user Scheduled Task registration fixed for executable and state paths containing spaces, validated in Windows CI. New owner pairing and signed device connection validated on Windows, and authenticated Linux end-to-end tests passed.

## Important limitations

- **First-install scripts are not independently publisher authenticated.** The release manifest authenticates the native updater, but the bootstrap PowerShell/POSIX scripts currently validate download integrity using SHA-256 sidecars fetched from the same GitHub release. For this release candidate, obtain binaries only from this repository's official release page and verify their signed release manifest with the separately trusted publisher public key (see [release publisher key](https://github.com/Phloraxx/infra-mcp/blob/fix/infra-ux-reliability-20261023/docs/RELEASE_PUBLISHER_KEY.md)). Do not pipe unreviewed scripts from arbitrary endpoints.
- **No live production deployment or automatic update:** you must set up a gateway, and owner approval must happen in the authenticated dashboard. No consumer-ready single-click installer is included.
- Physical Windows login/reboot, restricted-user policy, automatic upgrade/rollback and non-VPN public WSS ingress still need field validation. Do not migrate existing healthy agents automatically.
- These are pre-release developer/testing binaries without platform notarization or code-signing of each executable. Your OS may display untrusted publisher warnings. Do not disable security settings to bypass them.

Source and development history: https://github.com/Phloraxx/infra-mcp/pull/17
