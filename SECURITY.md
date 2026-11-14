# Security Policy

Infra MCP is remote-control software. Treat the gateway, device agent, privileged helper, protocol, installers, and update channel as security-sensitive components.

The v1 design intentionally supports arbitrary **user-level** shell commands and broad user-level filesystem access on authorized devices. Security therefore comes from strong authentication, explicit trust boundaries, privilege separation, transport security, auditing, and safe defaults—not from pretending the product is a read-only diagnostics service.

See [`docs/SECURITY_MODEL.md`](docs/SECURITY_MODEL.md) for the full design.

## Reporting a vulnerability

Use GitHub's **Report a vulnerability** flow for this repository. Private vulnerability reporting is enabled, so details go straight to the maintainer without a public issue.

If that flow is unavailable, open a minimal issue asking for a private contact path. Do **not** put exploit details in the public issue.

Never include in a report:

- device private keys, enrollment codes, gateway admin credentials or OAuth tokens;
- live hostnames, device names, IP addresses or unredacted audit logs;
- terminal output or file contents captured from a real machine.

Redacted logs and synthetic values are preferred.

Reports are especially useful for:

- bypassing device authentication, challenge-response binding or replay protection;
- enrollment code reuse, prediction or approval bypass;
- privilege escape from the user-level agent into the privileged helper or the host;
- unauthenticated or cross-device access to a session, terminal, job or file transfer;
- audit records that leak output, secrets or file contents;
- update artifacts accepted without a valid publisher signature;
- a destructive operation reachable through a read-only or reversible path.

## Current status

Infra v1 is implemented in this repository in Rust, and signed native release candidates have been published (see [docs/RELEASE_SIGNING.md](docs/RELEASE_SIGNING.md) and [docs/RELEASE_PUBLISHER_KEY.md](docs/RELEASE_PUBLISHER_KEY.md)). The live v0.3 deployment remains the production fallback until the cutover acceptance criteria pass; repository code and RC binaries are not production-deployed merely because they exist.

First-install bootstrap provenance and the public OAuth/TLS edge are still open security work.

## Non-negotiable requirements

- Never commit live credentials, private keys, OAuth secrets, passwords, session cookies, or `.env` files.
- All remote agent traffic must be encrypted in transit.
- Each paired device must have independently revocable credentials/identity.
- The normal device agent must run with ordinary user privileges by default.
- Elevated/root/SYSTEM operations must be isolated in the optional privileged helper.
- The gateway must authenticate ChatGPT/MCP clients and devices separately.
- Device private keys should be stored using platform-native protected storage where practical.
- Sensitive tool output must be treated as conversation-visible data.
- Audit logs should record metadata and outcomes without indiscriminately copying secrets, full file contents, or terminal output.
- Update artifacts must be authenticated before installation; rollback must be possible.
- Destructive operations must be clearly distinguishable from read-only and reversible operations in tool metadata and API semantics.

## High-impact areas requiring extra review

- privileged-helper IPC and authorization
- arbitrary command execution
- filesystem write/delete primitives
- file upload/download paths
- desktop input injection and accessibility automation
- screen capture and clipboard access
- agent auto-update
- device pairing and credential rotation
- gateway OAuth/token handling
- service/package/firewall/network operations
- container deletion, volume deletion, image pruning, or host mounts
- VM shutdown/reboot and disk operations

## Supported versions

Security fixes target the current release candidate line and `main`. Older release candidates are not patched; upgrade instead.

## If a credential may be exposed

Rotate or revoke it first, then investigate. A leaked device identity should be revoked at the gateway rather than rotated in place, and the device re-enrolled.
