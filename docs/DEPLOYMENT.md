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

Create a dedicated group and system user explicitly, then grant the user Docker access:

```bash
sudo groupadd --system infra-mcp
sudo useradd --system \
  --gid infra-mcp \
  --home /nonexistent \
  --shell /usr/sbin/nologin \
  infra-mcp
sudo usermod -aG docker infra-mcp
getent group infra-mcp
```

If the `infra-mcp` user or group already exists, skip the corresponding creation command rather than recreating it.

Record the numeric GID shown by `getent group infra-mcp`; configure that value as `INFRA_MCP_GID` for the MCP container so it can traverse `/run/infra-mcp` and access the Unix socket.

> Docker-group membership is effectively root-equivalent privilege on a typical Docker host. The security boundary is the host agent's fixed operation allowlist; do not expose the agent socket to unrelated containers or users.

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

Verify the socket exists and is owned by the intended user/group:

```bash
sudo ls -l /run/infra-mcp/agent.sock
```

## 5. Deploy the MCP gateway in Dokploy

Create a Dokploy Docker Compose deployment from this repository using `docker-compose.yml`.

Configure:

```text
AGENT_TOKEN=<same generated secret>
INFRA_MCP_GID=<numeric GID of infra-mcp group>
```

The Compose file publishes port 3000 only on the VM loopback interface by default. This is useful for local verification and a private tunnel. Do not change it to a public unauthenticated bind.

Verify locally on the VM:

```bash
curl http://127.0.0.1:3000/healthz
```

Expected response:

```json
{"status":"ok","service":"infra-mcp"}
```

If you later route the service through a Dokploy-managed domain, configure the domain for service `mcp-server` and container port `3000`. Dokploy can add the required Traefik routing/network configuration during deployment. Keep the MCP endpoint authenticated; a public hostname by itself is not an authentication boundary.

## 6. Connect ChatGPT

The safest first test is to keep the MCP endpoint private and use OpenAI Secure MCP Tunnel. Alternatively, add standards-compliant OAuth before publishing an HTTPS `/mcp` endpoint.

Once the MCP endpoint is reachable by ChatGPT:

1. In ChatGPT Web, keep Developer mode enabled.
2. Open **Settings → Apps** (or your workspace's **Workspace Settings → Apps**) and create/test the custom MCP app using the Developer mode controls available on your account.
3. Suggested app name: `Infra MCP`.
4. Suggested app description:

   `Securely inspect and operate my self-hosted infrastructure, including Linux host health, Docker containers and logs, Dokploy deployments, and OCI resources. Use read-only diagnostics first and request confirmation before consequential operations.`

5. Set the MCP server URL to your HTTPS `/mcp` endpoint, or use the configured Secure MCP Tunnel connection.
6. After the app connects, confirm that the advertised tools match the expected allowlist.
7. Keep consequential/write actions confirmation-gated while the integration is new.

OpenAI's current MCP availability differs by ChatGPT plan/workspace. Read-only MCP access may be available where full write/modify MCP actions are not. Verify that the account/workspace you use supports write actions before relying on tools such as `restart_container`, deploy, update, or reboot.

Test with read-only prompts first:

```text
Check my server health.
List my Docker containers.
Show the last 100 log lines from <container-name>.
Check for failed system services.
```

Only after those behave correctly, and only on a ChatGPT plan/workspace that permits MCP write actions, test the write tool:

```text
Restart <container-name>.
```

## 7. Before adding public OAuth

Do not treat the internal `AGENT_TOKEN` as ChatGPT user authentication. It protects only the local gateway-to-agent hop. For a public remote MCP deployment, use MCP-compatible OAuth and validate issuer, audience, expiry, and scopes on authenticated requests.

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
