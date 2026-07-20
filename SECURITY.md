# Security Policy

`infra-mcp` is infrastructure-control software. Treat every deployment as security-sensitive.

## Current status

The repository is an early scaffold and is **not ready to expose as an unauthenticated public MCP endpoint**.

The initial MCP gateway intentionally contains no arbitrary shell tool and does not mount the Docker socket. The host agent is still privileged through Docker-group membership and must be treated as trusted code.

## Deployment requirements

- Keep `AGENT_TOKEN` secret and use at least 32 random bytes of entropy.
- Never commit `.env` files or real credentials.
- Keep `/run/infra-mcp/agent.sock` accessible only to the intended service group/container.
- Do not expose the host agent over TCP.
- Do not mount `/var/run/docker.sock` into the MCP gateway.
- Do not add tools that accept arbitrary shell commands.
- Keep the MCP endpoint private until standards-compliant authentication is configured, or connect through a secure private tunnel.
- Use separate, least-privilege credentials for Dokploy and OCI integrations.

## Reporting vulnerabilities

Until a private security-reporting channel is configured for this repository, do not publish credentials, tokens, or live server details in a public GitHub issue. Rotate any credential that may have been exposed.

## High-impact changes

Changes involving any of the following require additional review before deployment:

- Arbitrary command execution.
- File read/write access outside explicitly allowed paths.
- Docker `exec`, privileged container creation, volume deletion, or host mounts.
- Package installation or OS upgrades.
- VM shutdown/reboot operations.
- Firewall, IAM, or networking changes.
- Database backup restoration or destructive database operations.
