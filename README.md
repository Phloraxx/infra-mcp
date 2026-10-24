# Infra MCP

Infra MCP is a personal multi-device control plane that lets ChatGPT operate authorized macOS, Windows, and Linux computers through MCP.

The product goal is not to be a thin SSH wrapper or a clone of Desktop Commander. The goal is a smaller, faster, more reliable, AI-native remote-control system with persistent terminals, structured system operations, resumable file transfer, multi-device routing, strong auditing, and eventually semantic desktop automation.

> **Current status:** **Infra v1 is actively implemented in Rust alongside the working v0.3 deployment.** The Rust control plane now covers signed device enrollment/authentication, persistent SQLite state/audit metadata, MCP `devices`/`exec`/`files`/`search`/`processes`/`jobs`/`terminal`/`transfer` plus structured `system`/`network`/`services`/`containers`/`git`, durable workers and PTYs, resumable binary transfer, native installers/startup adapters, `infra` status/doctor/repair commands, and a live embedded dashboard foundation. The live v0.3 runtime remains untouched until the benchmark/cutover criteria are met.

## Read this first (23 October 2026)

The **preferred near-term installer design** is [docs/ONE_COMMAND_SETUP.md](docs/ONE_COMMAND_SETUP.md), with broader pairing/UX design in [docs/ONBOARDING_REDESIGN.md](docs/ONBOARDING_REDESIGN.md). The underlying current-state review and infrastructure plan remain in [docs/CURRENT_REALITY.md](docs/CURRENT_REALITY.md) and [docs/NEXT_STEPS.md](docs/NEXT_STEPS.md).

**Next direction:** keep the existing Rust gateway, SQLite, outbound authenticated agents, and Android service. Make Tailscale optional rather than required: a private loopback gateway behind an existing HTTPS ingress, signed agents connecting over public `wss://` on 443, tested OAuth for MCP, and smooth installation/recovery. Measure persistent-connection latency and actual failures before introducing another transport or rewriting the UI. The feature branch is not production, and old fallback services stay until acceptance.

## Android companion — Infra Phone v0.3

The **Motorola Edge 60 Stylus** is enrolled as a first-class Android device over the existing signed Infra-MCP gateway. Live-verified v0.3 capabilities include scoped files, SHA-256-checked binary transfers, restricted SenseGuide development commands, user-reviewed APK installation and temporary, owner-enabled SenseGuide-only screenshot/tap access.

**Start with [Android current status](docs/ANDROID_CURRENT_STATUS.md)** for the validated capabilities, owner workflow, privacy and safety constraints, and outstanding reliability tests. Historic desktop/Oracle v0.3 references elsewhere in this README describe a **different legacy deployment**, not the Android app version.

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

## Implementation progress

The foundation and first persistent-control layer are implemented. See [`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md) for the exact current state.

For safe local end-to-end checks, `scripts/smoke-v1.sh` verifies signed enrollment/authentication, MCP execution and identity reconnect. `scripts/smoke-install-dashboard.sh` additionally exercises the embedded dashboard, one-time enrollment, native installer source mode, persisted local config/identity, online reconnect, `infra status`, `infra doctor`, sessions and activity APIs. Both use isolated temporary state and do not deploy or modify v0.3.

## License

No license has been selected yet. Do not assume third-party redistribution rights until a license is added.


## Android companion (planning track)

`apps/android-agent/` contains a separately buildable **v0.1 Android prototype** (device diagnostics, scoped local file API and short-lived private hosting). It does **not** yet directly enroll/connect to this Rust Infra gateway, or provide ADB/root/screen automation.

The first-class Android device roadmap calls for signed outbound WSS to the same gateway so ChatGPT sees the phone in the existing `devices` MCP tool without the temporary Oracle helper. See the [full plan](docs/ANDROID_AGENT_PLAN.md), [protocol adapter](docs/ANDROID_PROTOCOL_ADAPTER.md) and [security/acceptance checklist](docs/ANDROID_SECURITY_AND_TESTS.md).
