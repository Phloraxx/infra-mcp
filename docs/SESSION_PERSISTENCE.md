# Session persistence and connection reliability

The device connection and user sessions are different things. Temporary loss of the gateway connection must not automatically destroy useful work running on the device.

## Stable identifiers

Use separate IDs for:

- `device_id`: stable identity of the physical/logical device
- `boot_id`: changes after machine reboot
- `agent_instance_id`: changes when the controller process restarts
- `connection_id`: changes for every transport connection
- `session_id`: stable identifier for a PTY/job/transfer session

A newer authenticated connection for the same device may supersede a stale connection safely.

## Reconnection goals

The agent should automatically recover from:

- Wi-Fi changes
- IP changes
- brief Internet loss
- gateway restart
- laptop sleep/wake
- transport timeout
- agent controller restart

Use heartbeat/liveness checks, exponential backoff with jitter, and immediate reconnect attempts on strong local signals such as wake/network-change events.

## Duplicate/stale connection handling

Do not simply reject a second connection as `duplicate agent`.

The gateway should compare authenticated connection generations. A valid newer instance supersedes the old socket; pending operations attached to the stale connection are resolved as completed, failed, canceled, or `outcome_unknown` according to operation semantics.

## PTY persistence

Persistent terminals are first-class sessions.

A PTY should survive:

- an MCP request finishing
- ChatGPT/browser reload
- gateway disconnect/restart
- temporary device network loss

To survive a controller-agent restart, PTY ownership should be separable from the transport controller. One viable design is a session worker process (same signed binary in worker mode) that owns the PTY and a bounded on-disk/ring-buffer transcript while the main agent reconnects and reattaches through local IPC.

A normal machine reboot ends PTYs. After reboot they should be marked `interrupted_by_reboot`, not silently recreated.

## Job persistence

Long-running noninteractive jobs use durable metadata and output spooling on the device. Jobs should survive gateway/network loss and controller restart. After reboot, surviving OS processes can be re-associated only when identity can be proven safely; otherwise mark the job interrupted.

## Output semantics

Every streaming session should support:

- monotonically increasing offsets/sequence numbers
- bounded retained history
- attach from offset
- tail/latest reads
- cancellation/signals
- final exit status
- explicit truncation markers

The gateway must not pretend missing output was delivered.

## File-transfer persistence

Transfers should use chunk/range offsets and content hashes so interruption can resume without restarting multi-gigabyte transfers.

A final atomic rename/commit should occur only after integrity verification.

## Request retry safety

Do not blindly retry arbitrary execution after a disconnect. Each request should declare/idempotently derive retry behavior.

Possible result states include:

- succeeded
- failed
- canceled
- interrupted
- outcome_unknown

`outcome_unknown` is preferable to accidentally executing a destructive command twice.

## Gateway persistence

The gateway persists device/session metadata in SQLite, but device execution state lives on the device. A gateway restart should reconstruct session listings by reconciling with reconnected agents.

## User-visible behavior

If a device sleeps, the dashboard should show a transitional state rather than immediately implying failure. When it wakes and reconnects, surviving PTYs/jobs should reappear automatically.

If a session cannot resume, explain why (`device rebooted`, `worker exited`, `retention expired`, etc.).