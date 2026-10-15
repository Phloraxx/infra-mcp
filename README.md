# Infra MCP

Infra MCP is a personal multi-device control plane that lets ChatGPT operate authorized macOS, Windows, and Linux computers through MCP.

The product goal is not to be a thin SSH wrapper or a clone of Desktop Commander. The goal is a smaller, faster, more reliable, AI-native remote-control system with persistent terminals, structured system operations, resumable file transfer, multi-device routing, strong auditing, and eventually semantic desktop automation.

> **Current status:** design and migration phase for **Infra v1**. The existing TypeScript code in this repository is an older scaffold and should be treated as legacy/prototype reference. A separately deployed v0.3 prototype has already validated the ChatGPT → gateway → device-agent architecture with shell, files, process inspection, jobs, pairing, and automatic reconnect. The v1 implementation should be built cleanly rather than extending the Node agent indefinitely.

## Product principles

1. **Native and lightweight.** The agent should be one native binary with no Node/npm runtime dependency.
2. **Multi-device first.** One ChatGPT connection should control many authorized devices consistently.
3. **Persistent interaction.** PTYs, jobs, transfers, and sessions must survive individual MCP requests and temporary network interruptions where possible.
4. **Structured tools plus an escape hatch.** Prefer typed operations for common tasks, while retaining arbitrary user-level shell execution for flexibility.
5. **Privilege separation.** The normal agent runs as the signed-in user. Elevated operations go through a separate, optional privileged helper.
6. **AI-native desktop control.** Prefer accessibility/UI trees and semantic element targeting; use screenshots and coordinates as fallback mechanisms.
7. **Auditable by default.** Every consequential operation should have a durable, searchable audit record without leaking secrets.
8. **Easy to install, repair, update, and remove.** Reboot persistence and reconnection are product requirements, not manual setup notes.

## Target architecture

```text
                         ChatGPT
                            |
                     MCP Streamable HTTP
                            |
                            v
                  +---------------------+
                  |    infra-gateway    |
                  |        Rust         |
                  | OAuth + SQLite      |
                  | sessions + audit    |
                  +----------+----------+
                             |
                 persistent agent transport
                   WSS first; QUIC-capable
                             |
          +------------------+------------------+
          |                  |                  |
          v                  v                  v
   +-------------+     +-------------+    +-------------+
   | macOS agent |     |Windows agent|    | Linux agent |
   |    Rust     |     |    Rust     |    |    Rust     |
   +------+------+     +------+------+    +------+------+
          |                   |                  |
      optional            optional           optional
  privileged helper   privileged helper  privileged helper
```

The gateway is the MCP-facing control plane. Device agents make outbound authenticated connections to the gateway, so individual computers do not need public inbound ports.

## Planned tool surface

The MCP interface should remain compact. The target families are:

- `devices`
- `exec`
- `terminal`
- `files`
- `search`
- `processes`
- `system`
- `services`
- `network`
- `containers`
- `git`
- `desktop`
- `transfer`

Do not create dozens of tiny MCP tools when one coherent typed family is easier for the model to choose correctly.

## Technology direction

- **Rust** for the gateway and device agent.
- **Tokio** async runtime.
- **MCP Streamable HTTP** for ChatGPT ↔ gateway.
- **SQLite** for device metadata, sessions, jobs, audit events, and policy state.
- **WebSocket over TLS** for the first production agent transport, behind a transport abstraction.
- **QUIC/WebTransport** as an optional later transport after the protocol and resumability semantics are stable.
- Platform-native PTY, credential-store, accessibility, screen-capture, and service APIs behind Rust traits/modules.
- Optional **Tauri** tray/dashboard shell later; the headless agent must not depend on it.

Exact crates are implementation choices unless locked by `docs/DECISIONS.md`.

## Proposed v1 repository layout

```text
infra-mcp/
├── Cargo.toml
├── crates/
│   ├── protocol/            # versioned agent protocol types
│   ├── gateway/             # MCP, OAuth, device/session broker, SQLite
│   ├── agent/               # headless cross-platform agent
│   ├── platform/            # platform abstractions and implementations
│   └── privileged-helper/   # optional elevated helper
├── apps/
│   └── dashboard/           # optional web/tray UI when needed
├── docs/
└── legacy/                  # old prototype code after migration begins
```

Do not move/delete the current TypeScript code merely to match this layout. Preserve it until v1 reaches parity and the migration is explicitly performed.

## Start here

An implementation agent should read, in order:

1. [`AGENTS.md`](AGENTS.md)
2. [`docs/PRODUCT_SPEC.md`](docs/PRODUCT_SPEC.md)
3. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
4. [`docs/PROTOCOL.md`](docs/PROTOCOL.md)
5. [`docs/TOOLS.md`](docs/TOOLS.md)
6. [`docs/SECURITY_MODEL.md`](docs/SECURITY_MODEL.md)
7. [`docs/ROADMAP.md`](docs/ROADMAP.md)
8. [`docs/MIGRATION.md`](docs/MIGRATION.md)
9. [`docs/BENCHMARK.md`](docs/BENCHMARK.md)

Supporting documents:

- [`docs/PLATFORM.md`](docs/PLATFORM.md)
- [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md)
- [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md)
- [`docs/DECISIONS.md`](docs/DECISIONS.md)

## First implementation milestone

The first useful v1 milestone is deliberately smaller than the complete product:

- Rust workspace and protocol crate.
- Rust gateway with MCP health/device listing.
- Rust agent on Linux ARM64.
- authenticated pairing/device identity.
- `exec`, basic `files`, and `processes` parity.
- persistent reconnect.
- tests proving the v1 agent can run beside v0.3 without disrupting it.

Only after this foundation works should implementation expand into PTY sessions, resumable binary transfer, structured system modules, privilege helpers, and desktop automation.

## License

No license has been selected yet. Do not assume third-party redistribution rights until a license is added.
