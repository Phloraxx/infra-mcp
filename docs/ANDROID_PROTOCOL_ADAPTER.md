# Android ↔ Infra v1 protocol adapter specification

**Design status:** implementer-facing context, not an existing Android WebSocket client.

## Source of truth and compatibility

Current gateway contract is **Rust `crates/protocol/src/lib.rs`**, not generic MCP JSON-RPC. MCP is only between ChatGPT and the gateway. Android connects to `GET /agent` as an **Infra agent** using the same tagged Rust-Serde wire types as Linux/macOS/Windows.

Expected JSON enum encoding: `{"type":"auth_start","body":{...}}`, `{"type":"auth_challenge","body":{...}}`, `{"type":"signed_hello","body":{...}}`, `{"type":"hello_ack","body":{...}}`, `{"type":"request","body":{...}}` and `{"type":"response","body":{...}}`. All names are snake_case. Do not implement generic MCP messages, exchange an admin bearer token over this channel, or reuse the legacy bearer `legacy_hello` route for new Android clients.

Wire v1 constants (current code): `PROTOCOL_VERSION = 1`; maximum control message 8 MiB; gateway ping interval 10 seconds and stale inbound timeout 30 seconds. Existing desktop agent has its own idle watchdog; match heartbeat behavior, but keep Android scheduler/battery costs in mind.

## Initial pairing (existing gateway)

1. Owner requests enrollment in an authenticated dashboard session: `POST /api/enrollments`. Do not expose this endpoint or resulting code publicly.
2. APK locally generates Ed25519 keypair and random request UUID. It submits `POST /enroll` over authenticated TLS transport with:

```json
{
  "code": "<short-lived user-entered code>",
  "request_id": "<fresh UUID>",
  "device_name": "Android phone",
  "public_key": "<32-byte Ed25519 public key, base64url no padding>",
  "platform": "android",
  "arch": "aarch64",
  "agent_version": "0.2.0"
}
```

3. Gateway responds `{"device_id":"<stable-id>","device_name":"Android phone"}`. Save ID/name and encrypted private identity locally. Clear code from memory/UI when enrollment succeeds.
4. If gateway returns 401, 409, 410 or connection errors, surface concrete errors and preserve safe enrollment state. Do not silently regenerate identities after ambiguous results; support retry with the same pending request identity according to gateway semantics.
5. Unpair UX must invalidate locally cached identity, close socket, and request owner to revoke server registration. Local deletion alone does not revoke a stolen credential.

**Key storage:** AndroidKeyStore API does not offer a portable Ed25519 signing-key guarantee for every supported handset. Generate Ed25519 via a reviewed Java crypto implementation, wrap the 32-byte private seed at rest using a random-IV AES-256-GCM key stored under AndroidKeyStore. Check authenticated unwrapping, restore/backup exclusions, key invalidation and key-migration lifecycle. Never use unencrypted private seed in preferences, ZIP, ADB logs, crash reports, Gradle BuildConfig or Git. If an on-device secure Ed25519 signing provider exists, test it on actual supported devices before making it the default.

## Signed WebSocket connection

1. Client opens `wss://<configured-gateway>/agent` and verifies server certificate/hostname; no arbitrary self-signed trust or `noTLSverify`.
2. Client sends:

```json
{"type":"auth_start","body":{"protocol":1,"device_id":"<stable-id>","agent_instance_id":"<new-UUID-per-service-start>"}}
```

3. Gateway sends `auth_challenge` body with `nonce` (base64url text), `gateway_epoch` (UUID) and short expiration timestamp. The implementation expires within five seconds; sign promptly.
4. Construct `DeviceHello`:

```json
{
  "protocol": 1,
  "device_id": "<stable-id>",
  "device_name": "Android phone",
  "agent_version": "0.2.0",
  "platform": "android",
  "arch": "aarch64",
  "boot_id": "<stable-across-service-reconnect-until-reboot>",
  "agent_instance_id": "<same-UUID-from-auth_start>",
  "capabilities": ["status","system"],
  "metadata": {"android_api":"36"}
}
```

Boot ID: derive stable random installation-local marker for each OS boot where permitted, not device hardware identifier, serial, IMEI or advertising ID; avoid assuming Android allows readable Linux kernel boot ID. Do not rotate it at every socket reconnect.

5. Sign **exact Rust framing**, not JSON text. The `signed_auth_message` algorithm is:
   - Begin with ASCII bytes `infra-v1-device-auth\0`.
   - For each field, prefix raw value bytes with its **4-byte unsigned big-endian length**.
   - Fields in order: nonce as UTF-8 string, gateway epoch as **16 raw UUID bytes**, protocol as **2-byte big-endian integer**, device ID as UTF-8, agent-instance ID as **16 raw UUID bytes**, boot ID as UTF-8.
   - Sign with Ed25519 and encode the 64-byte signature base64url **without** `=` padding.
   - Rust test vectors should be exported to JSON and verified byte-for-byte by Java unit tests; beware UUID ordering, text encoding, and Java `ByteBuffer` big-endian defaults.

6. Send `{"type":"signed_hello","body":{"hello":{...},"signature":"..."}}`.
7. Gateway validates signature and returns `hello_ack` with protocol, connection generation, gateway epoch, 8 MiB limit and `reconcile_requested`. On mismatch or expiry, close and report.
8. Send `{"type":"reconcile","body":{"items":[]}}` immediately for MVP (no durable phone PTY/jobs yet).

## Request / response dispatch

Example request (hypothetical UUID and fields, for schema illustration only):

```json
{"type":"request","body":{"id":"11111111-1111-4111-8111-111111111111","operation":{"op":"status"}}}
```

Example success reply:

```json
{"type":"response","body":{"id":"11111111-1111-4111-8111-111111111111","result":{"status":"ok","body":{"device":"Android phone","android_api":36}}}}
```

Example explicit unsupported reply:

```json
{"type":"response","body":{"id":"11111111-1111-4111-8111-111111111111","result":{"status":"unsupported","operation":"exec"}}}
```

**Important:** `status` may have gateway-side formatting expectations. Before claiming parity, inspect actual `dispatch` and MCP `devices`, `system`, `files` conversion paths and test against the live-equivalent test gateway; structure success payloads to the exact operation-specific shape. The examples above show wire syntax, not confirmed status output schemas.

Use typed dispatch for `AgentOperation` and implement only requested, authorized, granted operations. Do not map unknown operation types into a shell or app intent. Unsupported or ungranted operations should yield `unsupported` or `unauthorized`; failed operations use `failed`. No output may imply success if denied, suspended or pending approval.

For every request:
- Validate protocol and UUID, size, age/deadline, concurrency, operation type, and user-authorized scopes.
- Attribute request to signed session and route through a bounded worker executor.
- Return same `id`; do not leak credentials, filesystem contents, phone numbers or notification text in logs.
- On reconnect, abandon or report uncertain mutations; **never replay an ambiguous write** just because a socket failed.
- For resumable transfers use the existing `TransferRequest` lifecycle; avoid >8 MiB frames, and verify SHA-256 before commit.
- For phone-screen actions requiring user permission, use an explicit second authorization state rather than treating pairing as blanket perpetual consent.

## Heartbeats and reconnect

- Reply promptly to gateway `{"type":"ping"}` with `{"type":"pong"}` and send reconcile if requested. Native WebSocket Ping/Pong control frames are separate.
- Treat client network change, VPN reconnect, airplane mode, idle timeout and gateway restart as normal; close obsolete socket and reconnect with jitter.
- Each process/service start uses fresh `agent_instance_id`; each reconnect gets a fresh gateway-assigned `connection_generation`.
- Keep a single active writer connection per device. When a newer socket authenticates, gateway supersedes the old generation. Never allow stale responses to complete new-generation operations.
- Pause or fail work cleanly when OS revokes permission or service stops.
- On explicit Stop: close socket, stop foreground notification, cancel queued requests and clear temporary in-memory data.
- On Unpair: stop service and wipe wrapped identity; server-side revocation is a separate mandatory owner step.

## Proposed module layout

```text
apps/android-agent/app/src/main/java/com/phloraxx/infraphone/
  MainActivity.java             existing v0.1 UI; refactor
  AgentService.java             existing v0.1 inbound HTTP; migrate/remove
  auth/IdentityStore.java       NEW, encrypted Ed25519 seed, stable identity
  auth/PairingClient.java       NEW, /enroll client
  protocol/ProtocolCodec.java   NEW, JSON wire and signing-byte construction
  transport/GatewaySocket.java  NEW, outbound WSS, certificate validation
  transport/AgentController.java NEW, lifecycle, status, retry and reconnect
  ops/AndroidDispatcher.java    NEW, permissions + unsupported responses
  ops/DeviceInfoOps.java        NEW, status/system
  ops/ScopedFilesOps.java       NEW, workspace and SAF grants
  ops/TransferOps.java          NEW, bounded chunks, hashing
```

The path tree is a *target*, not a claim those new Java files exist now.

## Golden interop fixtures and tests

Build producer tests in Rust `crates/protocol` with deterministic nonce, gateway UUID, hello UUID and boot ID. Export `signed_auth_message` bytes hex/base64 plus the full JSON handshake to versioned fixtures in `apps/android-agent/app/src/test/resources`. Verify in Java:
- complete byte-for-byte signing message;
- encoded public key length 32 / signature 64 bytes;
- base64url unpadded compatibility;
- JSON enum tag/case and UUID roundtrip;
- unknown/oversize frames rejected;
- wrong key, nonce/epoch, timeout, replay, revoked ID rejected by gateway;
- reconnect generation and one active socket;
- request dispatch exact IDs and honest `unsupported`.

Do not introduce divergent Android-specific protocol v2 without a Rust gateway migration/negotiation story.
