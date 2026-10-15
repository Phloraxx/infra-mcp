# Infra v1 architecture

## 1. Goal

Infra v1 is an AI-native remote computer control plane. A single ChatGPT MCP connection should be able to discover and operate multiple authorized macOS, Windows, and Linux machines with low latency and consistent semantics.

The architecture is deliberately split into a gateway/control plane and device-side execution agents.

## 2. High-level system

```text
                         ChatGPT
                            |
                  MCP Streamable HTTP + OAuth
                            |
                            v
              +-----------------------------+
              |        infra-gateway        |
              |-----------------------------|
              | MCP tool server             |
              | authentication              |
              | device/session broker       |
              | SQLite state                |
              | job/session metadata        |
              | audit log                   |
              | policy/confirmation hints   |
              +--------------+--------------+
                             |
                  outbound agent connections
                   WSS initially; pluggable
                             |
       +---------------------+---------------------+
       |                     |                     |
       v                     v                     v
+--------------+      +--------------+      +--------------+
| macOS agent  |      |Windows agent |      | Linux agent  |
| user context |      | user context |      | user context |
+------+-------+      +------+-------+      +------+-------+
       |                     |                     |
       +---- optional -------+------ optional -----+
             privileged local helper/service
```

Device agents make outbound connections to the gateway. No inbound port is required on a laptop/desktop.

## 3. Gateway responsibilities

The gateway owns:

- MCP Streamable HTTP endpoint.
- MCP client authentication/authorization.
- device registry and revocation state.
- pairing workflow.
- connection routing and supersession.
- logical stream/session registry.
- persistent metadata for jobs, sessions, transfers, and audit events.
- tool schemas and translation from MCP calls into protocol requests.
- bounded result shaping so models do not receive unnecessary output.

The gateway should **not** implement platform-specific shell/filesystem/process logic. It asks the agent to perform those operations.

The gateway should be restartable without permanently losing device registrations or durable jobs. SQLite is the default state store.

## 4. Device agent responsibilities

The normal agent runs as the logged-in/installed user and owns:

- shell and process execution in that user context.
- persistent PTY/ConPTY sessions.
- user-accessible filesystem operations.
- search, watch, tail, hashing, and file transfer.
- process inspection and signaling.
- structured Git/Docker/network/service operations that do not require elevation.
- desktop/accessibility operations available to that user/session.
- local capability discovery.
- reconnect, backoff, and protocol resumption.
- access to platform credential storage for device identity.

It is a headless native binary. A tray application may supervise or configure it, but core functionality must not require a GUI.

## 5. Privileged helper

A separate optional helper handles operations that require elevation.

```text
normal agent
    |
    | authenticated local IPC
    v
privileged helper
    |
    +-- service control requiring elevation
    +-- protected system files
    +-- package management
    +-- selected networking/firewall operations
    +-- other explicitly implemented admin operations
```

The helper must not expose a network listener. It should validate the caller and each requested operation. The normal agent must not simply forward an unconstrained root shell string to the helper by default.

User-level arbitrary shell is a product capability; arbitrary **root** shell is a separate, higher-risk design decision and should not be assumed.

## 6. Agent transport

### Initial production transport

Use WebSocket over TLS first because it is widely deployable and easy to proxy. Build transport behind an interface so higher-performance transports can be added later.

One physical connection carries multiple logical streams. The protocol, not the WebSocket API, defines stream IDs, sequence numbers, cancellation, offsets, and backpressure.

### Future transport

QUIC/WebTransport is desirable for independent native streams and reduced head-of-line blocking, especially when terminal traffic, file transfer, and desktop frames coexist. It should be added after protocol semantics are stable rather than delaying v1 parity.

## 7. Logical stream types

The protocol must support at least:

- RPC request/response
- terminal input/output
- job output
- file upload/download
- filesystem tail/watch events
- desktop frames/events

Streams are independently cancellable and resumable when the operation type allows it.

## 8. Identity and connection lifecycle

A device has stable identity independent of a particular socket.

Suggested identifiers:

- `device_id` — stable UUID/ULID assigned at pairing.
- `boot_id` — changes on OS reboot where available.
- `agent_instance_id` — changes each agent process start.
- `connection_generation` — monotonically increases when the gateway accepts a new connection for the device.

A newer valid connection should supersede a stale one. Do not reject a healthy reconnect merely because an old TCP/WebSocket connection has not timed out yet.

## 9. Persistent terminals

Interactive shells are long-lived resources, not giant `exec()` calls.

Conceptually:

```text
terminal.open -> terminal_id
terminal.write(terminal_id, bytes)
terminal.read(terminal_id, offset)
terminal.resize(terminal_id, cols, rows)
terminal.signal(terminal_id, signal)
terminal.detach(terminal_id)
terminal.attach(terminal_id)
terminal.close(terminal_id)
```

The agent maintains a bounded output ring/spool so the gateway can resume reads after reconnection. Session metadata may be mirrored in SQLite at the gateway.

## 10. Files and transfer architecture

Text file operations and raw binary transfer should be separate but coordinated primitives.

Required semantics include:

- line and byte-range reads
- atomic write/replace
- exact block edit and patch
- copy/move/mkdir
- trash vs permanent delete
- glob/grep/tree/search
- watch/tail
- hash
- binary upload/download with chunking
- offset-based resume
- integrity verification

Large files must never be embedded wholesale into one JSON response.

## 11. Structured system modules

Infra should provide structured operations for frequently used domains such as processes, services, network listeners, containers, Git, and system resource inspection.

These modules may call OS tools internally when that is the simplest reliable implementation, but they should return normalized structured results and avoid making the model build fragile command pipelines for routine tasks.

## 12. Desktop architecture

Desktop control should be layered:

1. accessibility/UI automation tree
2. window/application metadata
3. semantic actions against elements
4. screenshot/screen regions for visual reasoning
5. coordinate mouse/keyboard fallback

Platform implementations:

- macOS: Accessibility APIs plus native screen/input APIs.
- Windows: UI Automation plus Windows capture/input APIs.
- Linux desktop: AT-SPI where available, with environment-specific fallbacks.

Desktop control is not required for the first parity milestone.

## 13. Persistence

Gateway SQLite should eventually contain tables equivalent to:

- devices
- device_credentials/public_keys
- connections
- pairing_requests
- terminal_sessions
- jobs
- transfers
- audit_events
- policies/settings
- schema_migrations

Do not store terminal/file payloads in SQLite by default. Use bounded spool files/object storage where payload persistence is needed.

## 14. Failure semantics

Operations are classified as:

- **idempotent/retryable** — e.g. status read.
- **resumable** — e.g. file download by offset.
- **persistent** — e.g. background job or PTY session.
- **non-idempotent** — e.g. command that changes state.

If connectivity fails after dispatch of a non-idempotent operation and the outcome cannot be proven, return an `outcome_unknown` state. Never silently repeat a potentially consequential command.

## 15. Repository transition

The desired Rust workspace should be introduced alongside the existing TypeScript scaffold. Existing code remains available as reference until parity is proven. See `MIGRATION.md`.
