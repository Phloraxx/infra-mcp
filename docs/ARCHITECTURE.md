# Architecture

## Goal

`infra-mcp` lets ChatGPT inspect and operate infrastructure through explicit MCP tools while avoiding a direct arbitrary shell, direct SSH exposure, or a Docker socket mounted into the Internet-facing MCP process.

## Components

### 1. MCP gateway

The MCP gateway is the only component ChatGPT talks to. It should run as an unprivileged container and expose Streamable HTTP at `/mcp`.

Responsibilities:

- Advertise narrowly scoped MCP tools.
- Validate user/model inputs before forwarding them.
- Mark read-only and consequential tools with MCP annotations.
- Call the host agent over an authenticated Unix socket.
- Later, call Dokploy and OCI APIs using separately scoped credentials.

The gateway must not:

- Mount `/var/run/docker.sock`.
- Receive arbitrary shell commands.
- Store root credentials.
- Expose internal agent credentials in MCP tool output.

### 2. Host agent

The host agent runs directly on the Linux VM under systemd. It listens only on `/run/infra-mcp/agent.sock`; there is no TCP listener.

The initial agent supports only:

- Host health/status.
- Docker container listing.
- Bounded Docker log reads.
- Failed systemd service inspection.
- Restarting one validated, explicitly named container.

Commands are launched using `execFile(command, args)` with fixed binaries and fixed argument shapes. User input is never passed to a shell.

The service account is placed in the Docker group so it can inspect and restart containers. Docker-group membership is effectively a high-privilege capability, so the agent remains part of the trusted computing base even though it does not run as UID 0.

### 3. Dokploy integration (planned)

Deployment operations should use Dokploy's API/webhooks instead of reproducing deployment logic with low-level Docker commands.

Proposed tools:

- `list_projects`
- `deployment_status`
- `deploy_project`
- `redeploy_project`

Use a dedicated Dokploy API credential with the narrowest permissions available.

### 4. OCI integration (planned)

OCI APIs are the preferred path for cloud-level lifecycle and recovery actions.

Proposed tools:

- `instance_status`
- `start_instance`
- `stop_instance`
- `reboot_instance`
- `vulnerability_report`
- `security_findings`

Use a dedicated OCI principal and IAM policy. Do not give the MCP gateway tenancy-wide administrator permissions.

OCI Run Command should be retained as an emergency recovery path when the local host agent is unavailable, not as the primary path for interactive log retrieval and routine diagnostics.

## Trust boundaries

```text
Untrusted / model-controlled input
            |
            v
+---------------------------+
| MCP Gateway               |
| - schema validation       |
| - tool allowlist          |
| - no Docker socket        |
| - no arbitrary shell      |
+-------------+-------------+
              |
              | Unix socket + internal bearer token
              v
+---------------------------+
| Host Agent                |
| - fixed operations only   |
| - fixed binaries/args     |
| - Docker-group privilege  |
+-------------+-------------+
              |
              v
       Docker / systemd
```

A compromise of the MCP gateway should not automatically grant arbitrary shell execution. However, because the gateway is allowed to request consequential agent operations, authentication and tool-level confirmation remain mandatory parts of the production design.

## Authentication

### Development

Keep the MCP endpoint private. Preferred options:

1. OpenAI Secure MCP Tunnel.
2. A local/private endpoint during development.

Do not put the current no-auth `/mcp` endpoint directly on a public hostname.

### Production

Implement MCP-compatible OAuth using an established identity provider. Validate access tokens in the MCP gateway, including issuer, audience, expiration, and scopes.

Suggested scopes:

- `infra:read` — diagnostics, status, logs.
- `infra:operate` — routine restarts and deployments.
- `infra:admin` — VM lifecycle, security updates, or other high-impact operations.

Keep the host-agent bearer token separate from user OAuth. The agent token is internal service-to-service authentication and must never be returned through MCP.

## Rollout

### Phase 1 — read-heavy diagnostics

- `system_status`
- `list_containers`
- `container_logs`
- `failed_services`

Validate ChatGPT connectivity and audit behavior before adding more write tools.

### Phase 2 — routine operations

- `restart_container`
- Dokploy deploy/redeploy operations
- Restart selected system services through explicit allowlists

### Phase 3 — cloud and security

- OCI instance lifecycle
- OCI security/vulnerability findings
- Patch/update inspection
- Explicit, confirmation-gated security update workflows

### Phase 4 — recovery

- OCI Run Command for repairing/restarting the host agent
- Out-of-band MCP hosting if the primary VM must be controllable while powered off

## Rules for adding tools

Before adding an MCP tool:

1. Decide whether it can be read-only.
2. Use a strict schema and bounded output.
3. Avoid accepting command strings, paths, URLs, or environment-variable names unless absolutely necessary.
4. Prefer an upstream API over shell commands.
5. If a command is necessary, use a fixed executable and an argument array; never concatenate into a shell command.
6. Mark write/consequential behavior accurately in MCP annotations.
7. Log the operation without logging credentials or sensitive output.
8. Add tests for rejected/malicious inputs.
