# Security Policy

Infra MCP is remote-control software. Treat the gateway, device agent, privileged helper, protocol, installers, and update channel as security-sensitive components.

The v1 design intentionally supports arbitrary **user-level** shell commands and broad user-level filesystem access on authorized devices. Security therefore comes from strong authentication, explicit trust boundaries, privilege separation, transport security, auditing, and safe defaults—not from pretending the product is a read-only diagnostics service.

See [`docs/SECURITY_MODEL.md`](docs/SECURITY_MODEL.md) for the full design.

## Current status

Infra v1 is not implemented yet. Existing TypeScript code is prototype/legacy material and should not be exposed as a new public production service merely because the repository documentation now describes v1.

A separate v0.3 deployment exists and remains the fallback during migration.

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

## Vulnerability handling

Do not post secrets or live infrastructure details in a public issue. If a credential may have been exposed, rotate/revoke it first and investigate second.

A private security-reporting mechanism should be enabled before this project is promoted for third-party use.
