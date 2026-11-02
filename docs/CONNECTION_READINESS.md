# Fresh native connection verification

Implemented in a stabilization candidate, 2 November 2026. **Not deployed.**
Uses the existing signed WebSocket, native agent and lifecycle CLI. No additional
listener, pairing credential or owner token is required for the local check.

## Why cached online state is insufficient

Previously setup/repair accepted `runtime.state == "online"` plus a live agent
PID. A paused/stalled controller or a stale runtime file could satisfy that
condition. Gateway `/healthz` is a separate signal and cannot prove that this
device's signed connection is responsive.

## Opt-in status roundtrip

Native agents advertise `connection.readiness_v1`. Only signed connections with
that capability receive the extension. Existing native/Android peers retain the
unchanged Ping/Pong exchange; no new frames are sent to a non-advertising peer.

1. After registration, and every ten-second heartbeat, the gateway sends
   `connection_probe` with a random UUID nonce and the connection generation.
2. The agent answers through its priority control queue with
   `connection_probe_reply`: the probe, controller PID and protocol version.
3. The gateway consumes a matching unexpired challenge only once, checks the
   live routing map still selects that generation, and returns `connection_ready`.
4. The agent accepts only its outstanding matching probe/status within ten
   seconds. It queues a local receipt for a single coalescing writer outside the
   socket receive loop. Disk delays retain the original receipt time.

Each side retains one outstanding exchange, with a ten-second lifetime. The
gateway does not create an ordinary pending RPC or execute/retry a command.
The reply confirms the controller's PID/protocol status and active routing; it
does not certify desktop permissions, all tool capabilities or job availability.

## Local receipt and CLI behavior

`connection-proof.json` is a versioned diagnostic receipt. It contains PID,
device ID, gateway epoch, agent-instance ID, connection generation, endpoint
SHA-256 fingerprint, probe start time and verification time. It contains no key/token and does not
copy the configured endpoint URL. It is local user state, not an independent OS
attestation; a process running as that user can alter local diagnostics.

Runtime state adds optional gateway-epoch/agent-instance fields. Old runtime JSON
still decodes. The agent clears those fields outside online state. A receipt
must match all identifiers, the configured endpoint and a live agent process.
Its timestamp must not be in the future and must be at most 30 seconds old.

- `infra status --json` remains a snapshot with the normal startup inspection.
  Its new `connection` object distinguishes verified, stale, missing, unreadable,
  mismatched and offline evidence. Plain output labels runtime state as recorded.
- `infra status --verify --json` waits for a probe begun after this check and its matching confirmation.
  Default wait: 15 seconds; `--timeout-ms` accepts 1..60000. Failure exits nonzero
  with `connection.verified: false` and `connection_verification_failed`.
  This mode skips external startup inspection and returns `startup: null`;
  ordinary status still returns its existing startup object. Local filesystem
  and OS process-query failure limits are not a whole-machine deadline guarantee.
- Setup, startup-enabled legacy install and repair require a newly confirmed
  receipt from a probe begun after the check starts. Healthy Windows reruns can keep the same PID and generation while
  waiting for its next roundtrip. `--no-startup` still only configures/pairs and
  explicitly skips startup; it does not claim an online connection.
- Reconnect/update waits additionally require a changed process/connection.
  An unrelated runtime timestamp change is no longer sufficient. Gateway epoch
  and instance changes identify reconnects even if a restarted gateway's numeric
  generation decreases.
- Doctor reports connection verification independently from process, startup,
  credential-storage history and gateway HTTP health. It does not read native
  signing keys to obtain connection evidence.

## Compatibility and remaining gates

Deploy gateway support before the native CLI/agent pair. New agents still connect
to old gateways, and old agents still connect to new gateways, but they cannot
produce the new receipt without both sides supporting the extension. Strict
verification reports unverified with upgrade guidance; it never falls back to
claiming readiness from cached state. Android readiness receipts are not added.

This affects updater health gates too. A rollback to a pre-extension agent may
restore a running binary while remaining unverified by the new CLI. Explicit
version compatibility and physical failed-update rollback acceptance remain
open. No rollback certification follows from this change.

Normal status/doctor still use existing platform startup commands; their complete
OS-command timeout behavior is a separate remaining task. Physical Windows
login/reboot/sleep/network tests and public OAuth/WSS acceptance also remain open.
No live gateway/device or excluded host is used by the candidate tests.

## Evidence

Unit tests reject incorrect/expired/replayed probes, stale routing generations,
old/mismatched/future receipts and cached online-only waits. A real loopback smoke
verifies a signed native agent, then pauses that owned agent: its PID and stored
online state remain, but a fresh CLI check fails. Windows hosted smoke checks an
already-running setup rerun retains PID/generation while producing fresh evidence.
Hosted results and physical-device results must be recorded separately.
