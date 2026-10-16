# AGENTS.md — Infra v1 implementation context

This file is the starting context for coding agents working in this repository.

## Mission

Build **Infra v1**: a native, lightweight, cross-platform remote-control system optimized for ChatGPT/MCP. It must eventually replace both the separately deployed Node-based Infra v0.3 prototype and Desktop Commander for the owner's normal workflows.

This is a personal-use product first. Prefer reliability, low complexity, low tool-call overhead, and excellent installation/recovery behavior over enterprise ceremony.

## Important repository state

- The current TypeScript source predates the v1 product direction.
- Do **not** treat old README/security constraints such as “never allow arbitrary shell” as v1 requirements.
- Arbitrary **user-level** shell execution is an intentional v1 capability.
- Do not delete old code until the Rust implementation has parity and migration is explicitly requested.
- Do not modify or deploy the live v0.3 installation merely because code in this repo changes.

## Required reading

Before implementing a substantial feature, read:

1. `docs/PRODUCT_SPEC.md`
2. `docs/ARCHITECTURE.md`
3. `docs/PROTOCOL.md`
4. `docs/TOOLS.md`
5. `docs/SECURITY_MODEL.md`
6. `docs/ROADMAP.md`
7. `docs/MIGRATION.md`
8. `docs/BENCHMARK.md`
9. `docs/DECISIONS.md`

Platform work must also read `docs/PLATFORM.md`.

## Locked direction

Unless the user explicitly changes the direction:

- Agent: Rust native binary.
- Gateway: Rust.
- Async runtime: Tokio.
- Gateway persistence: SQLite.
- ChatGPT transport: MCP Streamable HTTP.
- Agent transport for initial implementation: authenticated WebSocket/TLS behind a transport abstraction.
- Protocol must support multiplexed logical streams even when physically carried over one WebSocket.
- Device identity should evolve from bearer tokens to per-device asymmetric keys.
- Normal agent runs as the user; elevated operations belong in a separate optional helper.
- Headless use is mandatory. A tray/Tauri UI is optional and later.
- Desktop automation should prefer semantic accessibility/UI APIs before coordinate clicking.

## Engineering rules

### Keep the MCP surface compact

Prefer a small number of coherent tool families (`exec`, `terminal`, `files`, `system`, etc.) over dozens of single-purpose tools. Common operations should return structured JSON. Shell is the fallback, not the implementation for every structured tool.

### Separate control plane from execution plane

The gateway should route, authenticate, persist metadata, expose MCP, and broker streams. OS-specific execution belongs on the device agent.

### Preserve stream semantics

Do not design terminal output, file transfer, desktop frames, or long-running jobs as one giant request/response buffer. Protocol primitives must allow incremental events, offsets, cancellation, resumption, and backpressure.

### Never make reconnect ambiguous

Every device connection needs a stable `device_id` and an ephemeral connection/boot instance. A newer authenticated connection must be able to supersede a stale socket safely. Operations whose outcome becomes unknown after a disconnect must report that uncertainty rather than being silently retried.

### Prefer atomic file mutation

Writes and patches should be atomic whenever practical. Destructive delete and recoverable trash should be distinct actions.

### No hidden privilege

The regular agent must not quietly become root/SYSTEM. The privileged helper is a separate process/service and a separate trust boundary.

### Do not put secrets in logs or Git

Never commit real pairing credentials, private keys, OAuth secrets, machine tokens, passwords, or live environment files. Audit metadata should identify actions while avoiding command output or file contents unless explicitly configured.

## Current Rust workspace

Implementation has begun. Preserve and extend this workspace:

```text
Cargo.toml
crates/
  protocol/
  gateway/
  agent/
  platform/
  local-state/
  cli/
  privileged-helper/   # planned; not created yet
```

Suggested dependency categories, not mandates:

- serialization: `serde`, `serde_json`
- async: `tokio`
- HTTP/WebSocket: `axum` + compatible WS stack
- tracing: `tracing`, `tracing-subscriber`
- SQLite: `rusqlite` or `sqlx` with SQLite
- IDs: UUID/ULID
- crypto: well-reviewed Ed25519/TLS libraries
- PTY: `portable-pty` or direct platform layer if needed
- file notifications: `notify`

Do not adopt a heavy framework solely because it is convenient for one platform.


## Current implementation checkpoint

As of 2026-10-16 the branch contains working SQLite persistence/audit metadata, one-time Ed25519 enrollment/challenge authentication, MCP `devices`/`exec`/`files`/`search`/`processes`/`jobs`/`terminal`/`transfer` plus structured `system`/`network`/`services`/`containers`/`git`, durable workers, persistent PTYs, reconnect reconciliation, native bootstrap installers/startup adapters, an `infra` lifecycle/doctor CLI, and an embedded dashboard foundation. Linux ARM64 has a real side-by-side user-service burn-in proving fresh PTY I/O across gateway and controller restarts. A real Mac burn-in now verifies cross-device enrollment/exec plus fresh same-PTY I/O and same-job continuation across controller restart, fresh same-PTY I/O after gateway restart, and zero leaked launchd worker registrations/processes afterward; macOS uses self-cleaning independent launchd workers, stable boot identity, and bounded blocking control sockets. Windows ConPTY startup-query/close/shutdown handling now passes the full Windows CI suite and native x64 artifact build; only real-device Windows acceptance remains gated. Device identity metadata is now v2: signing keys migrate into the native OS credential store when available, while headless Linux exposes an explicit private-file fallback through `infra doctor`. Gateway liveness now retires stale sockets through protocol heartbeats/RPC timeout and refreshes session reconciliation periodically, so online/session state must not be inferred from socket existence alone. Private-mode gateway access is authenticated: MCP uses the admin bearer credential and the browser exchanges it for a restart-stable HMAC-signed HttpOnly session with CSRF protection; public standards-compatible OAuth remains pending. Real Mac acceptance has migrated the v1 identity into Keychain-backed `native_keyring` metadata with no private key on disk and successfully reconnected. `infra doctor` must not read native secret bytes; the agent records only a non-secret storage/verification timestamp after successful signed authentication. Abandoned `starting` PTY/job metadata without a worker PID expires after a startup grace so failed spawns cannot remain active forever. Read `docs/IMPLEMENTATION_STATUS.md` before choosing the next task; do not recreate these features.

The live Node v0.3 deployment remains separate and must not be modified as a side effect of v1 development.

## Testing expectations

Every protocol change should have tests for:

- successful request/response
- streaming output
- cancellation
- disconnect during operation
- reconnect/supersession
- malformed/oversized frames
- unauthorized device
- protocol version mismatch

Cross-platform behavior needs CI or real-device smoke tests for:

- Linux ARM64
- macOS ARM64
- Windows x64

Linux x64 and Windows ARM64 are desirable later.

## Migration discipline

Infra v0.3 is currently useful. v1 development must be side-by-side until replacement criteria in `docs/BENCHMARK.md` are met.

Do not perform a destructive cutover as part of an implementation PR.

## Definition of “better than Desktop Commander”

It is not enough to match a checklist. Infra should require fewer model tool calls for normal work, reconnect more reliably, cover more of the owner's devices, use substantially fewer resources, provide stronger session persistence, and expose a more coherent MCP interface.

If a proposed feature makes the product significantly harder to install, repair, or understand, challenge the design before implementing it.
