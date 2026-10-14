# Infra MCP

A personal device-control product: one MCP endpoint and web console, with small background agents on your computers.

The active implementation is in [`runtime/`](runtime/). The older `apps/` scaffold is retained as historical work and is not the runtime deployed on Oracle.

## Current release: 0.3.0

- Oracle-hosted web console with Oracle selected by default, device switching, file editing and saved background-job output.
- Password sign-in for the web console and OAuth/PKCE for ChatGPT.
- One-time pairing codes and separate, revocable device credentials.
- macOS/Linux shell installer and Windows PowerShell installer. User installations need no sudo or administrator access; an official, checksum-verified private Node runtime is downloaded when required.
- Persistent device identity, single-instance agents, WebSocket reconnect backoff and heartbeat checks.
- Background jobs retain logs and exit status across an agent restart. Rebooted jobs are reported as interrupted; commands are not blindly replayed.
- Six MCP tools: `devices`, `shell`, `files`, `process`, `jobs`, `status`.

Desktop automation and standalone Android control are **not implemented in this release**. The UI and capability reporting make that limitation explicit.

## Run the gateway

Use Node.js 22 or newer:

```bash
cd runtime
npm ci --omit=dev
```

Provide `PUBLIC_BASE`, `OAUTH_USER`, `OAUTH_PASSWORD` and `TOKEN_SIGNING_SECRET` through your service environment. `TOKEN_SIGNING_SECRET` must remain stable across restarts so existing connections and dashboard sessions remain valid. `PORT` defaults to 8787; the gateway binds to loopback. Put it behind an HTTPS reverse proxy or tunnel.

```bash
npm start
```

The web console is at `/`, and the MCP endpoint is `/mcp`. Connect ChatGPT using OAuth authentication. The existing Oracle deployment uses `https://mcp.phloraxx.us.to`.

`AGENT_TOKEN` is supported only as a compatibility path for existing 0.2 agents. New installations use per-device pairing. Avoid distributing the legacy shared token.

## Install a device

Sign in to the console, choose **Add device**, select an OS and copy the installation command. The code expires in ten minutes and can be used once. Device names must be unique. Reinstallation reuses the saved identity unless a new code is explicitly supplied.

### Startup and permission modes

| Mode | Startup | Permission scope |
|---|---|---|
| macOS user | LaunchAgent after user login | That user's access |
| macOS `--system` | LaunchDaemon at boot, after any required disk unlock | System agent; no interactive desktop |
| Linux user | systemd user service; boot availability requires lingering | That user's access |
| Linux `--system` | systemd system service at boot | Root shell; no interactive desktop |
| Windows user | Per-user Scheduled Task, with Run-key fallback | That user's access |
| Windows `-System` | Startup task as SYSTEM | System shell; no interactive desktop |

Administrator installation does not automatically create a usable GUI session. Sleep, powered-off machines, disk-unlock screens and OS permission restrictions still affect availability.

Unix installer options: `--base URL`, `--code CODE`, `--name NAME`, `--system`, `--manual`, `--uninstall`. For system mode, download the script first and run it with sudo. Windows equivalents are `-Server`, `-Code`, `-Name`, `-System`, `-Uninstall`.

Uninstall removes startup configuration, retains local configuration/job logs, and tells the user to revoke the device in the console.

## Operation and efficiency

The agent needs no npm dependencies on Node 22+. GUI helpers and local models are not started. Gateway state is a small JSON store; no additional database server or message broker is required.

Short commands return bounded output (16,000 characters by default). Long commands should run with `background: true`; their ID can be used with `jobs.read`. Logs are capped at 8 MiB per job. Up to 20 jobs can run concurrently; the most recent 50 finished jobs are retained when another job is started.

Device tokens stay in local files with restricted permissions. Dashboard responses exclude credentials. The gateway routes commands only to the named, authenticated device. A dropped connection does not prove that a command failed: inspect the job before retrying.

## Tests and known gaps

```bash
cd runtime
npm ci
npm test
npm run test:ui  # requires a Playwright browser
npm run test:install  # Linux user-systemd integration; uses isolated test services
```

See [`docs/PRODUCT_TEST_STATUS.md`](docs/PRODUCT_TEST_STATUS.md) for measured coverage and outstanding hardware checks. Backend/agent integration CI passed on Windows, macOS and Linux; Windows installer syntax and dashboard UI CI also passed. Hardware installation and reboot checks remain separate requirements.
