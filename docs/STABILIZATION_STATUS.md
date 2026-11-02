# Reliability stabilization

Updated 2 November 2026. **Source changes, not a production deployment.**

## Source baseline

The stabilization branch combines Studio `11dc229`, the newer installer/test
changes through `4233a36`, and the timeout fix from `92e760c` (PR #23). Existing
device identities, wire protocol version, and persisted session formats are
preserved. Validation uses disposable local fixtures and hosted CI; live
production agents and gateways are outside this effort's execution scope.

## Implemented

- Request expiry no longer retires an otherwise healthy connection. Heartbeats,
  socket failures, and authenticated generations remain authoritative.
- Queue admission and response waiting share one deadline. Saturated queues
  report that the operation was not dispatched, rather than waiting forever.
- A synchronous drop guard removes pending waiters when a caller is cancelled.
  Responses must match the originating device and connection generation. A
  disconnect releases that generation's waiters with an uncertain outcome.
- One-shot exec and structured subprocess tools share bounded asynchronous
  output collection. The deadline includes pipe EOF, so descendants holding
  inherited output pipes cannot hang the caller indefinitely.
- Unix commands own a process group; Windows commands own a Job Object.
  Timeout/cancellation terminates that owned boundary. Cleanup has a separate
  two-second allowance and reports when termination cannot be confirmed.
- A single non-macOS worker-reaper thread collects exited children without
  tying active durable workers to the controller's lifetime. macOS retains its
  independent launchd worker mechanism.

`process-wrap` supplies safe cross-platform group/job ownership; application code
does not add unsafe process/handle code. Output retention limits remain bounded.
Structured command timeouts retain captured partial output. The existing exec
wire result still reports `deadline_exceeded` without partial stdout/stderr;
extending that contract needs a separate compatible protocol change.

## Verification

Initial local macOS ARM64 results, before further client-contract changes:

- 76 baseline workspace tests passed after source reconciliation.
- 86 workspace tests passed after reliability changes (39 agent, 32 gateway,
  and 15 CLI/local-state/platform/protocol tests).
- Rust formatting and strict workspace/all-target Clippy passed.
- `scripts/smoke-v1.sh` passed against a temporary loopback gateway/agent,
  covering enrollment/authentication/reconnect and MCP execution, files,
  search, system/network, Git, and resumable transfer.

New regressions cover queue saturation, abort before/after dispatch, socket loss,
connection-bound responses, inherited output pipes, bounded output retention,
and descendant cancellation. Linux CI additionally exercises 100 worker exits;
Windows CI exercises its Job Object deadline path. CI and physical Windows
results must be recorded separately; local macOS success is not Windows proof.

## Remaining gates

MCP schema/error/discovery ergonomics, pagination/images, native setup readiness,
localized Windows task status, update/rollback acceptance, bounded dashboard
requests, cross-client coverage, and physical sleep/login/reboot tests remain
separate work. No claim of production readiness or deployed incident resolution
follows from these code changes.

## Fresh connection readiness

The separate readiness slice replaces cached-online/PID success with a fresh
nonce/status roundtrip on the exact signed connection selected by the gateway.
Native receipts bind process/device, epoch, instance, generation and endpoint
fingerprint. They expire after 30 seconds; lifecycle waits require a probe begun
after the check starts and its matching confirmation. A single coalescing disk writer stays outside the socket
loop. `status --verify` skips external startup queries and returns a failing
connection result if no fresh receipt arrives. Ordinary status/doctor separate
connection evidence from startup, credential history and HTTP health.

Older/non-advertising peers retain legacy heartbeats. Strict checks need both
gateway and native agent support; rollout order and pre-extension rollback
compatibility remain explicit gates. See [CONNECTION_READINESS.md](CONNECTION_READINESS.md).
