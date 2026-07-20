# Deployment guide

This guide describes the intended first deployment on a Linux Oracle Cloud VM that already runs Docker and Dokploy.

## 1. Prepare the repository on the host

Clone the repository into `/opt/infra-mcp` and build the host agent:

```bash
sudo git clone https://github.com/Phloraxx/infra-mcp.git /opt/infra-mcp
cd /opt/infra-mcp
sudo npm install
sudo npm run build
```

For repeatable production deployments, add and commit a lockfile once dependency validation is complete, then use `npm ci` instead of `npm install`.

## 2. Create the host-agent service account

Create a dedicated system user and add it to Docker's group:

```bash
sudo useradd --system --home /nonexistent --shell /usr/sbin/nologin infra-mcp
sudo usermod -aG docker infra-mcp
getent group infra-mcp
```

Record the numeric GID shown by `getent group infra-mcp`; configure that value as `INFRA_MCP_GID` for the MCP container so it can access the Unix socket.

> Docker-group membership is effectively high privilege. The security boundary is the host agent's fixed operation allowlist; do not expose the agent socket to unrelated containers or users.

## 3. Configure the internal agent token

Generate a random secret:

```bash
openssl rand -hex 32
```

Create the environment directory and file:

```bash
sudo install -d -m 0750 /etc/infra-mcp
sudo sh -c 'umask 077; cat > /etc/infra-mcp/host-agent.env'
```

Add:

```text
AGENT_TOKEN=<generated-secret>
AGENT_SOCKET_MODE=0660
```

The MCP gateway must receive the same `AGENT_TOKEN` through Dokploy's environment configuration. Never commit the real value.

## 4. Install the systemd service

```bash
sudo cp /opt/infra-mcp/deploy/systemd/infra-mcp-agent.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now infra-mcp-agent
sudo systemctl status infra-mcp-agent
```

Verify the socket exists:

```bash
sudo ls -l /run/infra-mcp/agent.sock
```

## 5. Deploy the MCP gateway in Dokploy

Create a Dokploy application/Compose deployment from this repository using `docker-compose.yml`.

Configure:

```text
AGENT_TOKEN=<same generated secret>
INFRA_MCP_GID=<numeric GID of infra-mcp group>
```

The Compose file publishes port 3000 only on the VM loopback interface by default. Do not simply change it to a public unauthenticated port.

Verify locally on the VM:

```bash
curl http://127.0.0.1:3000/healthz
```

Expected response:

```json
{"status":"ok","service":"infra-mcp"}
```

## 6. Connect ChatGPT

The safest first test is to keep the MCP endpoint private and use OpenAI Secure MCP Tunnel. Alternatively, add standards-compliant OAuth before publishing an HTTPS `/mcp` endpoint.

Once the MCP endpoint is reachable by ChatGPT:

1. In ChatGPT Web, keep Developer mode enabled.
2. Open **Settings → Plugins** and create a developer-mode app.
3. Suggested app name: `Infra MCP`.
4. Suggested app description:

   `Securely inspect and operate my self-hosted infrastructure, including Linux host health, Docker containers and logs, Dokploy deployments, and OCI resources. Use read-only diagnostics first and request confirmation before consequential operations.`

5. Set the MCP server URL to your HTTPS `/mcp` endpoint, or select the configured secure tunnel connection.
6. After the app connects, confirm that the advertised tools match the expected allowlist.
7. Set the app permission level to ask before making changes while the integration is new.

Test with read-only prompts first:

```text
Check my server health.
List my Docker containers.
Show the last 100 log lines from <container-name>.
Check for failed system services.
```

Only after those behave correctly, test the write tool:

```text
Restart <container-name>.
```

## 7. Before adding public OAuth

Do not treat a static API key as a ChatGPT authentication solution. ChatGPT MCP integrations use MCP-compatible OAuth for user authorization; the gateway should validate issuer, audience, expiry, and scopes on every authenticated request.

Recommended scope split:

```text
infra:read
infra:operate
infra:admin
```

Use an established identity provider rather than writing an authorization server from scratch.

## 8. Next integrations

After the base path is proven:

1. Add Dokploy API integration for deploy/redeploy/status.
2. Add OCI SDK integration with a least-privilege OCI principal.
3. Add OCI vulnerability/security findings as read-only tools.
4. Add update inspection before any update-installation tool.
5. Keep OCI Run Command as an emergency recovery path for a failed host agent.
