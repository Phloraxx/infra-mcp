# Agent protocol

This document defines protocol semantics, not a final wire encoding. Keep transport and message semantics separable.

## 1. Goals

The gateway↔agent protocol must support:

- reliable RPC
- multiple concurrent logical streams
- incremental output
- cancellation
- durable jobs and PTYs
- resumable transfers
- clean reconnect/supersession
- bounded memory use
- protocol version negotiation
- explicit unknown-outcome states

## 2. Transport

Initial implementation: WebSocket over TLS.

Do not tie business logic directly to a specific WebSocket library. Define a transport trait/interface so QUIC/WebTransport can be added later without rewriting session logic.

Even on WebSocket, multiplex independent logical streams at the protocol layer.

## 3. Identity

A paired device possesses an Ed25519 asymmetric keypair generated locally; this is implemented in the v1 agent/gateway.

Conceptual pairing flow:

1. dashboard/gateway creates a short-lived one-time pairing code.
2. agent generates a device keypair locally.
3. agent submits code, public key, requested device name, platform metadata, and a pairing request ID.
4. gateway validates/consumes the code and registers a stable `device_id`.
5. agent stores private identity using protected local storage when possible.
6. future connections authenticate possession of that device key.

A legacy bearer-token path remains only as an explicitly named development/migration fallback; signed challenge authentication is the normal v1 path.

## 4. Connection handshake

Agent hello should include fields conceptually equivalent to:

```json
{
  "protocol": 1,
  "device_id": "...",
  "device_name": "...",
  "agent_version": "...",
  "platform": "linux|darwin|windows",
  "arch": "arm64|x86_64",
  "boot_id": "...",
  "agent_instance_id": "...",
  "capabilities": ["exec", "files", "pty"],
  "resume": {"last_gateway_epoch": "..."}
}
```

Gateway hello-ack returns negotiated protocol version, connection generation, server epoch, limits, and any session-resume instructions.

A valid newer connection may supersede the previous connection for the same `device_id`.

## 5. Message envelope

Every logical message should have a common envelope similar to:

```json
{
  "v": 1,
  "kind": "request|response|event|data|cancel|ack|error",
  "id": "message-id",
  "stream_id": "optional-stream-id",
  "seq": 42,
  "body": {}
}
```

Do not depend on timestamps for ordering; use sequence numbers/offsets where ordering matters.

## 6. RPC

RPC request fields:

- request ID
- operation name
- typed payload
- optional deadline
- idempotency/retry metadata where useful

Response states should distinguish:

- `ok`
- `failed`
- `canceled`
- `deadline_exceeded`
- `outcome_unknown`
- `unsupported`
- `unauthorized`

`outcome_unknown` is essential for non-idempotent operations interrupted after dispatch.

## 7. Streaming

Streams need:

- `stream_id`
- monotonically increasing byte/event offset or sequence
- bounded receiver windows/backpressure
- end-of-stream marker
- cancellation
- optional resume position

Never assume that “one request = one complete payload.”

## 8. Terminal semantics

PTY session state lives primarily on the agent.

Operations:

- open
- write input
- read from offset
- resize
- signal
- metadata/status
- detach
- attach/resume
- close

Agent should spool a bounded amount of output to disk or ring storage. Reads use explicit offsets. The gateway can therefore recover after a temporary disconnection without duplicating the full terminal transcript.

A terminal session should have both a session ID and the underlying process identity.

## 9. Job semantics

Jobs differ from PTYs:

- normally non-interactive
- durable metadata
- stdout/stderr spool
- explicit completion status
- cancel/signal support

A job started successfully should continue even if the MCP caller disconnects, unless the request explicitly asks for attached lifetime semantics.

## 10. File transfer

Large binary transfer must be chunked and resumable.

Metadata includes:

- transfer ID
- source/destination path
- total size when known
- current offset
- chunk size
- hash algorithm and expected hash when known
- temp/staging destination

Upload should write to a staging file and atomically rename into place after verification where the filesystem permits.

Download resume uses byte offsets and may verify prefix/full hashes as appropriate.

Do not base64 huge files inside ordinary RPC JSON.

## 11. Desktop stream

Desktop automation may generate:

- screenshots/frame payloads
- changed-region metadata
- accessibility tree snapshots/deltas
- input/action acknowledgements

The protocol should allow binary payload channels without forcing the gateway to persist raw frames.

## 12. Limits

Gateway and agent negotiate limits such as:

- maximum control-frame size
- maximum concurrent streams
- maximum outstanding RPCs
- chunk size
- spool limit per PTY/job
- idle session timeout defaults

Oversized inputs fail explicitly. Do not truncate structured request data silently.

## 13. Versioning

Protocol changes are versioned independently from application versions.

Rules:

- backward-compatible optional fields may be added within a protocol version.
- incompatible semantics require a new negotiated protocol version.
- agents should fail with a clear `protocol_mismatch` rather than connecting partially.

## 14. Reconnect and resume

After reconnect:

- gateway identifies sessions it still tracks.
- agent reports locally alive terminals/jobs/transfers.
- both sides reconcile IDs and offsets.
- stale unknown sessions are marked appropriately.

Never automatically replay a non-idempotent RPC purely because the connection dropped.

## 15. Observability

Each operation should carry trace/request identifiers so gateway and agent logs can be correlated without logging the sensitive payload itself.

## 16. Implemented v1 handshake and reconciliation

The current Rust implementation uses the following signed path:

1. agent sends `auth_start` with protocol, stable device ID, and fresh agent-instance ID;
2. gateway loads the enrolled public key and rejects unknown/revoked devices;
3. gateway sends a fresh random `auth_challenge` bound to the gateway epoch and a short expiry;
4. agent signs a deterministic length-prefixed authentication message containing the nonce, gateway epoch, protocol, device ID, agent instance and boot ID;
5. agent sends `signed_hello`;
6. gateway verifies Ed25519 possession and returns `hello_ack` with connection generation;
7. agent immediately sends its active job/terminal reconciliation snapshot.

Enrollment codes are short-lived and stored hashed. The gateway retains an explicitly separate legacy bearer-token handshake only for migration/development. Non-idempotent requests are not automatically replayed after connection loss.

The current PTY/job streams use byte-offset reads over typed RPC rather than the future general multiplexed binary-stream envelope. Binary/resumable transfer is still pending.

### Current transfer RPC mapping

The first implementation maps resumable transfer semantics onto bounded control RPCs: `upload_begin`, `upload_chunk`, `upload_commit`, `download_begin`, `download_chunk`, `status`, `list`, and `cancel`. Chunks are capped so their base64 representation remains comfortably below the control-frame limit. The persistent transfer ID and byte offsets provide resume semantics now; a later dedicated binary stream transport may replace base64 chunks without changing the higher-level lifecycle.
