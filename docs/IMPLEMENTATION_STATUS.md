# Infra v1 implementation status

This file records what exists in code. Product intent remains in the other design documents.

## 2026-10-15 — foundation branch

Implemented:

- Rust workspace with `protocol`, `gateway`, `agent`, and `platform` crates.
- Protocol version 1 hello/ack and request/response types.
- Explicit operation outcomes including `outcome_unknown`.
- Linux boot ID / cross-platform platform identity abstraction.
- Rust gateway health endpoint.
- Authenticated agent WebSocket handshake with protocol-version validation.
- Connection generations: a newly authenticated connection for the same device supersedes the older gateway record.
- Online device registry.
- Bounded one-shot user-level `exec` operation with timeout and 1 MiB stdout/stderr capture limits.
- Agent automatic reconnect with bounded exponential backoff.
- Rustls/WebPKI support for `wss://` gateway connections.
- MCP Streamable HTTP endpoint with initial `devices` and `exec` tools.
- REST development endpoints for health/device inspection and exec smoke testing.
- JSON structured tracing.
- Cross-platform Rust CI definition for Linux/macOS/Windows.
- Protocol, platform, exec, deadline, and bounded-output unit tests.

Not implemented yet:

- asymmetric device enrollment/identity
- SQLite persistence
- durable jobs
- files/process tools
- PTY/session workers
- binary transfer
- installer/startup integration
- dashboard
- privileged helper
- desktop automation
- production OAuth

## Verification note

The workspace builds, passes clippy with warnings denied, and passes unit tests on the Oracle Linux ARM64 development host.

A local long-lived second agent process was intentionally not launched through the current remote-control connector because that connector's safety layer blocked spawning another persistent remote agent. Instead, the gateway test suite now starts an isolated local server and simulated WebSocket agent to verify authenticated handshake, registration, request/response exec round trips, and newer-connection supersession automatically.

The existing live Node v0.3 installation was not changed.
