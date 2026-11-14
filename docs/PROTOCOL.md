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

The opt-in `connection.readiness_v1` extension adds nonce/status probe and
confirmation frames only for signed agents advertising support. It preserves
legacy Ping/Pong and protocol version 1; see [connection readiness](CONNECTION_READINESS.md)
for frame semantics, bounded state, receipt binding and compatibility limits.

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

The current PTY/job streams use byte-offset reads over typed RPC rather than the future general multiplexed binary-stream envelope. Transfer chunks can already travel in capability-gated bulk frames (section 18); a general multiplexed stream envelope is still pending.

### Current transfer RPC mapping

The first implementation maps resumable transfer semantics onto bounded control RPCs: `upload_begin`, `upload_chunk`, `upload_commit`, `download_begin`, `download_chunk`, `status`, `list`, and `cancel`. Chunks are capped at 256 KiB. The persistent transfer ID and byte offsets provide resume semantics; a later dedicated binary stream transport may replace the chunk RPCs without changing the higher-level lifecycle.

## 17. Additive directory-list pagination (2 November 2026)

`FileAction::List` retains `{"action":"list","path":"..."}`. Optional
`offset` (u64 entry index) and `length` (entry count) are omitted on serialization
when absent. Older JSON remains decodable and the protocol version stays 1.
Both gateway and native agent validate explicit bounds (offset <=100,000;
length 1..1,000). The agent also limits scan count and serialized page bytes.

Because an old parser may silently ignore added optional fields, the gateway
must check `files.list_page` on the exact live connection selected for dispatch
before sending either field. Cached registry capabilities do not authorize it.
Missing capability yields a non-retryable `unsupported` tool error before queue
admission; no request or pending waiter is created. Requests without either
field retain the old operation. New results add continuation/count/order fields
without removing `entries` or `truncated`. No new session, spool, daemon or
automatic retry is introduced; normal cancellation/generation rules apply.

## 18. Additive bulk binary frames (14 November 2026)

Payload-bearing operations can carry their bytes in a binary WebSocket frame
instead of base64 inside JSON. Control stays JSON, `PROTOCOL_VERSION` stays 1,
and every existing JSON shape is unchanged.

- Capabilities: `transfer.binary_v1` and `terminal.binary_v1`, advertised in
  `DeviceHello.capabilities`. `HelloAck.binary_frames` (serde default `false`)
  tells the agent the gateway will use them on this connection. An agent that
  does not advertise them, or a gateway that does not acknowledge them, keeps
  the base64 path exactly as before.
- Frame layout, all integers big-endian: magic `I` (u8), version 1 (u8), kind
  (u8: 0 `upload_chunk`, 1 `download_chunk`, 2 `terminal_data`, 3 reserved),
  flags (u8), the transfer or terminal id as 16 raw UUID bytes, offset (u64),
  payload length (u32), then the payload. The header is 32 bytes; the
  improvement plan's "16-byte header" wording conflicts with its own field list,
  and the field list is what both sides implement. Nothing inside a bulk frame
  is JSON.
- Pairing: a payload frame always follows a JSON announcement on the same
  socket, so request/response correlation, the pending registry and the audit
  trail are unchanged. Uploads and terminal input announce with
  `upload_chunk_binary` / `write_binary` and then send the payload; a download
  chunk answers with JSON metadata (`data_binary: true`, `length`) and then the
  payload frame.
- Rejection: a binary frame on a connection that did not negotiate bulk frames,
  a malformed frame, a frame without its announcement, or a frame that
  disagrees with its announcement is a protocol error and closes the socket. A
  frame whose request already timed out is dropped and the relay resumes from
  the last confirmed offset.
- The MCP surface is unchanged: `transfer` still takes and returns base64, and
  the gateway re-encodes once at the edge. The cross-device `copy` relay moves
  raw chunks from the source frame straight into the destination frame.
