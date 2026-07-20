# infra-mcp

Secure MCP gateway for observing and controlling self-hosted infrastructure from ChatGPT. The project is designed to provide narrowly scoped, auditable access to Linux system diagnostics, Docker workloads, Dokploy deployments, and Oracle Cloud Infrastructure (OCI) without exposing an unrestricted shell or the Docker socket directly to the public MCP service.

> **Status:** Initial scaffold. Start with read-only diagnostics and container logs, then add carefully scoped write operations after the deployment and authentication path is verified.

## Architecture

```text
ChatGPT Web
    |
    | MCP over HTTPS / Secure MCP Tunnel
    v
MCP Gateway (unprivileged container)
    |
    | authenticated Unix socket
    v
Host Agent (systemd service, allowlisted operations)
    |
    +-- Docker / container logs
    +-- systemd / journal
    +-- host health and diagnostics
    |
    +----------------------+
                           |
MCP Gateway ---------------+-- Dokploy API
                           +-- OCI APIs
```

The MCP gateway never receives arbitrary shell access and does not mount `/var/run/docker.sock`. Privileged host operations are isolated behind a small host agent with explicit allowlists and strict input validation.

## Initial capabilities

- System status: uptime, CPU load, memory, and root filesystem usage.
- Docker: list containers and fetch bounded container logs.
- Docker actions: restart a named container as an explicit write operation.
- systemd: inspect failed services.
- Health endpoints for the MCP gateway and host agent.

Planned integrations include Dokploy deployments, OCI instance lifecycle controls, OCI Vulnerability Scanning/Cloud Guard findings, package/security update workflows, and emergency recovery through OCI Run Command.

## Repository layout

```text
infra-mcp/
├── apps/
│   ├── mcp-server/       # ChatGPT-facing MCP gateway
│   └── host-agent/       # Privileged host-side allowlisted agent
├── deploy/
│   └── systemd/          # Host-agent service unit
├── docs/
│   ├── ARCHITECTURE.md
│   └── DEPLOYMENT.md
├── .env.example
├── docker-compose.yml
├── package.json
└── tsconfig.base.json
```

## Security model

1. **No arbitrary shell tool.** MCP tools map to explicit operations only.
2. **No Docker socket in the MCP container.** Docker access stays in the host agent.
3. **Unix-socket boundary.** The MCP gateway talks to the host agent through `/run/infra-mcp/agent.sock`.
4. **Shared agent token.** Requests over the local socket still require an internal bearer token for defense in depth.
5. **Bounded inputs.** Container names are validated and log output is capped.
6. **Read vs. write annotations.** MCP tools advertise read-only/destructive intent so ChatGPT can apply confirmation policies appropriately.
7. **Secrets stay outside Git.** Runtime secrets belong in Dokploy/environment configuration.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for trust boundaries and [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) for the Oracle VM + Dokploy setup and ChatGPT connection steps.

## Development

Requirements:

- Node.js 22+
- npm 10+
- Docker for the MCP gateway image
- Linux + systemd for the host agent deployment

```bash
npm install
npm run build
```

Run the host agent locally:

```bash
cp .env.example .env
npm run dev:agent
```

Run the MCP server in another terminal:

```bash
npm run dev:mcp
```

The MCP endpoint is exposed at:

```text
http://localhost:3000/mcp
```

For ChatGPT, deploy behind HTTPS or use OpenAI Secure MCP Tunnel. Do not expose a no-auth development endpoint directly to the public Internet.

## ChatGPT app metadata

Suggested app name:

```text
Infra MCP
```

Suggested app description:

```text
Securely inspect and operate my self-hosted infrastructure, including Linux host health, Docker containers and logs, Dokploy deployments, and OCI resources. Use read-only diagnostics first and request confirmation before consequential operations.
```

## Deployment direction

The intended production layout for the current Oracle VM + Dokploy setup is:

- `mcp-server`: deployed as a normal Dokploy application/container.
- `host-agent`: installed as a systemd service on the VM host.
- `/run/infra-mcp`: bind-mounted into the MCP container.
- ChatGPT: connected to the `/mcp` endpoint as a developer-mode app.
- Authentication: add standards-compliant OAuth before exposing the MCP endpoint publicly; alternatively use a private Secure MCP Tunnel where appropriate.

## Why not OCI Run Command for everything?

OCI Run Command is useful as an emergency/fallback path, but everyday diagnostics such as container logs are better served by the low-latency host agent. The long-term design keeps OCI APIs for VM lifecycle/recovery and uses the local agent for interactive host operations.

## License

No license has been selected yet. Add one before encouraging third-party reuse or contributions.
