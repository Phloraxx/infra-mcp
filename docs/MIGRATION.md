# Migration from Infra v0.3 / legacy code

## Purpose

Infra v1 is a clean replacement architecture, but the working Node prototype contains useful behavior that should be preserved deliberately rather than rediscovered.

This document captures the observed baseline as of 2026-10-15. Exact runtime numbers may change.

## Deployed v0.3 prototype baseline

The separately deployed prototype currently uses Node.js and a small JSON-over-WebSocket agent/gateway design.

Observed components:

- Node 22 runtime
- Express gateway
- `ws` WebSocket transport
- MCP TypeScript SDK
- device agent with shell/files/process/jobs capability families
- on-disk background job metadata/output
- pairing codes and per-device bearer tokens
- heartbeat/reconnect behavior
- Linux/macOS/Windows installer scripts
- startup via systemd user service, launchd, or Windows scheduled/login startup depending on platform
- simple `infra` CLI for status/reconnect/logs/repair on Unix-like installs

The deployed v0.3 code is not guaranteed to match the older TypeScript scaffold currently committed in this repository.

## Important current limitations

Observed design limitations include:

- Node runtime dependency on each client.
- one-shot shell execution buffers command output rather than providing a real PTY.
- command timeout is bounded and unsuitable for persistent interactive tools.
- WebSocket has a small control payload ceiling in the current implementation.
- text file reads are bounded and binary files lack a first-class streaming transfer API.
- current search is a custom recursive scanner with fixed scan/file-size limits.
- process behavior is partially implemented through parsing platform command output.
- job model is useful but separate from true interactive terminal sessions.
- device identity is based on stored bearer tokens rather than asymmetric per-device keys.
- no semantic desktop/UI control.
- no separate privileged helper.

## What v1 must preserve

Do not regress these useful prototype qualities:

### Low-friction pairing

The pairing code workflow is easy to understand. Keep the UX, strengthen the identity underneath it.

### Outbound agent connection

Clients do not need inbound public ports. Keep this.

### Multi-device routing

One gateway can address multiple devices by name/identity. This is central to the product.

### Automatic reconnect

Backoff/reconnect is already expected behavior. v1 must make it more robust, not remove it.

### Durable jobs

The existing job idea—start work, retrieve output later, cancel it—is valuable. Integrate it into a more general session/stream model.

### Simple local CLI

Commands equivalent to `infra status`, `reconnect`, `logs`, and `repair` are worth preserving.

### Side-by-side startup persistence

The existing system demonstrated that agents can persist after login/reboot on all three primary OS families. v1 installation must meet or exceed that reliability.

## What should be replaced

- Node/npm/private-Node bootstrapping → native binary releases.
- ad-hoc JSON RPC assumptions → versioned protocol with logical streams.
- whole-buffer shell execution → stream-capable exec and real PTY.
- custom recursive content scan → efficient search implementation.
- binary-file workaround → resumable transfer primitive.
- bearer device token as final identity → asymmetric device identity.
- ambiguous duplicate socket rejection → connection generations and supersession.
- main-agent elevation temptation → separate privileged helper.

## Migration strategy

### 1. Do not rewrite the live deployment in place

Build and test v1 under separate service names, configuration directories, and ports/endpoints.

Suggested temporary naming:

- `infra-v1-gateway`
- `infra-v1-agent`
- config/state under a distinct v1 directory

Do not reuse the v0.3 job/state directory during early development.

### 2. Canary on Linux server first

The Linux ARM64 server is the easiest place to run both agents simultaneously and compare:

- connection uptime
- shell latency
- filesystem behavior
- jobs
- process inspection
- resource use

### 3. Add macOS

Validate launchd persistence, sleep/wake reconnect, PTY behavior, key storage, and eventually Accessibility permissions.

### 4. Add Windows

Validate ConPTY, Scheduled Task/service startup options, path semantics, process tree cancellation, and protected credential storage.

### 5. Tool compatibility

MCP tool names do not need to remain identical to v0.3. Prefer the cleaner `TOOLS.md` model, but maintain behavioral parity before removing the old service.

## Rollback

During migration:

- v0.3 remains installed and disabled/enabled independently.
- v1 has separate credentials/state.
- a v1 failure must not corrupt v0.3 state.
- cutting back to v0.3 should require only restarting/re-enabling the existing service and selecting the old MCP connector.

## Cutover rule

Do not remove Desktop Commander or v0.3 merely because v1 “works.” Perform the benchmark in `BENCHMARK.md` and close must-win gaps first.

## V1 implementation checkpoint — 2026-10-15

The Rust v1 branch now has SQLite-backed device records, one-time Ed25519 enrollment and challenge authentication, `exec`, file/process parity, durable job workers, real PTY/ConPTY workers, connection supersession, and reconnect reconciliation. The old shared bearer-token path exists only as an explicit development/migration fallback.

The implementation continues to use a separate state directory/database and has not altered the deployed v0.3 service. Startup installers and cutover are still pending.

### Current side-by-side v1 product layer

The v1 branch now has its own local state format, `infra` CLI, startup adapters, bootstrap installers and embedded dashboard. These use distinct v1 state (`~/.infra-mcp-v1` by default) and must continue to coexist with v0.3 during canary testing. Do not repoint/remove the live connector until the real-device reliability and benchmark gates pass.
