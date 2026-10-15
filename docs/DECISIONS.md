# Architecture decisions

This is a lightweight ADR index. “Locked” means implementation should follow it unless the user deliberately changes the direction.

## D001 — Rebuild v1 core in Rust

**Status:** Locked

Both the gateway and device agent move to Rust. The existing Node implementation remains a migration reference/fallback.

Reason: native distribution, lower runtime dependency burden, stronger control over processes/PTYs/streams, and one cross-platform systems codebase.

## D002 — Arbitrary user-level shell is a core feature

**Status:** Locked

The old restricted allowlist-only security model does not match the product goal. Authorized user-level command execution is intentionally supported.

Elevation remains separate.

## D003 — Separate privileged helper

**Status:** Locked

Do not run the normal agent permanently as root/SYSTEM merely to enable occasional admin operations.

## D004 — Compact MCP surface

**Status:** Locked

Expose coherent tool families instead of a large collection of tiny tools. Structured common operations coexist with shell escape hatch.

## D005 — MCP gateway uses SQLite

**Status:** Locked for personal v1

SQLite is sufficient for device/session/job/audit metadata. Do not introduce Postgres/Redis unless a proven requirement appears.

## D006 — WSS first, transport abstraction from day one

**Status:** Locked

Ship the initial agent channel over WebSocket/TLS for deployability. Protocol semantics must not depend on WebSocket so QUIC/WebTransport can be added later.

This supersedes the earlier idea of making QUIC mandatory before parity.

## D007 — Persistent PTY is foundational

**Status:** Locked

Interactive terminal sessions are first-class resources with IDs, offsets, attach/detach, and signals. They are not emulated by repeated one-shot commands.

## D008 — Per-device asymmetric identity is the target

**Status:** Locked direction

Pairing remains simple, but long-lived device authentication should use a locally generated keypair rather than one reusable bearer token as the final design.

## D009 — Semantic desktop automation first

**Status:** Locked direction

When desktop control is implemented, accessibility/UI automation is preferred over coordinate-only clicking. Visual screenshots are complementary.

## D010 — Headless core, optional tray UI

**Status:** Locked

Servers must run the same core agent without a GUI. A Tauri/native tray application may improve desktop UX later but is not part of the execution core.

## Open implementation choices

These remain intentionally open until prototypes/tests justify a choice:

- exact Rust MCP SDK/library
- exact WebSocket/HTTP stack within the Rust ecosystem
- `rusqlite` vs async SQLite wrapper
- exact PTY crate vs direct OS code
- exact secure-store crate/bindings
- exact update-signing framework
- exact macOS privileged-helper/native shim approach
- whether/when QUIC becomes default transport
