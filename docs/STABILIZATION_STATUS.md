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

The following stacked MCP-contract slice is implemented in source: action enums,
field/shell/limit descriptions, native MCP failure flags and recovery fields,
native screenshot image blocks with JSON compatibility, malformed-exec response
classification, and rejection of ineffective directory-list pagination options.
An added HTTP smoke verifies discovery and error results using disposable local
state. Full pagination, granular capability metadata and actual client-side image
acceptance remain open.

The dashboard-recovery slice adds a ten-second budget covering API response
headers and body reads, visibly stale last-confirmed data, and explicit uncertainty
for timed-out mutations without replay. Auth and sign-out requests also use the
bounded helper. The primary Windows copy command reuses the existing unique-temp,
exit-checking, cleanup-capable bootstrap generator. Browser fixtures passed both
stalled-header and stalled-body recovery plus a mutation that reached the server
but never returned a response; existing terminal/auth/responsive tests passed.

The initial reliability PR's hosted Linux x64/ARM64, macOS, Windows (including
installer smoke), and dashboard jobs passed. This is hosted CI evidence, not
physical Windows login/reboot or a production deployment.

MCP schema/error/discovery ergonomics, pagination/images, native setup readiness,
localized Windows task status, update/rollback acceptance, bounded dashboard
requests, cross-client coverage, and physical sleep/login/reboot tests remain
separate work. No claim of production readiness or deployed incident resolution
follows from these code changes.
