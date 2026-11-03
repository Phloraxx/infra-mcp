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
classification, and validation of directory-list pagination options.
An added HTTP smoke verifies discovery and error results using disposable local
state.

The following pagination slice adds native entry-offset pages, a deterministic
whole-directory filename order, continuation/count metadata, a 1,000-entry page
cap, a 512 KiB response cap and a 100,000-name scan cap. Explicit pagination is
gated on `files.list_page` from the exact live connection, so legacy/Android
agents cannot silently ignore the new optional wire fields. Their no-options
list remains supported. Offsets are not snapshots under directory mutation.
Tests cover traversal beyond the old cap, byte-shortened continuation, invalid
limits, symlinks, Linux lossy filename collisions, legacy wire shape and stale
registry capabilities. Product smoke exercises actual signed-agent MCP pages.
Broader granular capability/readiness metadata and actual client-side image
acceptance remain open; Android pagination is not implemented by this slice.

The dashboard pagination slice uses 100-entry requests when `files.list_page`
is advertised, with Previous/Next history built from actual continuation
offsets. It renders one page at a time, retains the last confirmed page on error,
ignores responses from old routes/devices and clears paths/history on sign-out
or device switch. Old agents retain a no-options list with an honest incomplete
listing message. Browser tests cover keyboard/mobile controls, byte-shortened
pages, failure/recovery, capability loss, stale responses and private-state
clearing. These offsets still are not directory snapshots.

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

## Native installer transaction

Bootstrap adapters now delegate paired CLI/agent activation to the staged native
CLI. Installation/profile locks serialize lifecycle mutations; a hashed journal,
backups and commit marker support interrupted activation and rollback without
copying identity keys. Startup commands reuse the bounded subprocess runner.
Windows signed updates report pending until the post-exit native activation
records its result. See [INSTALL_TRANSACTION.md](INSTALL_TRANSACTION.md) for
acceptance, compatible-release requirements and unresolved physical-device gates.

The installer slice passes 106 local workspace tests, strict Clippy, formatting,
the disposable installer/dashboard smoke and owned-file rollback acceptance.
Hosted Windows rollback and combined-candidate results must be recorded separately.


## Release version integrity

The follow-up release slice rejects signed update downgrades relative to the
running CLI and requires manifest/request/binary version agreement. Same-version
repair and internal transaction rollback remain supported. Release automation
checks tags/build metadata and exact platform inventory/checksums before signing,
and records the tagged source commit. See [RELEASE_SIGNING.md](RELEASE_SIGNING.md)
for replay limits, fixture acceptance and first-install distribution gates.


## 3 November release candidate

`v0.1.1-rc.1` assembles the stabilization and pairing-completion work with
actionable dashboard connection checks, readable mobile navigation, an original
ChatGPT Images sign-in asset, and guarded startup ownership across profiles.
Native artifacts and bootstrap defaults share the same SemVer. Promotion requires
green integration checks, a complete signed release, a fresh SQLite backup, an
isolated copied-database smoke and a guarded gateway rollout with rollback.
Gateway replacement does not upgrade enrolled agents or certify OS signing, first
bootstrap provenance or physical lifecycle behavior. Live deployment requires
explicit session authorization; ordinary development remains on owned fixtures.
