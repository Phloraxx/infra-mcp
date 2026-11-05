# Benchmark and replacement criteria

Infra v1 is intended to replace Desktop Commander, so “feature complete” is comparative.

## Observed baseline — 2026-10-15

On the same Oracle Linux ARM64 host during an informal comparison:

- custom Infra/remote-desktop agent successfully provided shell, file, and process access.
- Desktop Commander provided shell, file, process, interactive-session, richer search, and document-oriented tools.
- both reached the same user-level filesystem when connected as the same OS user.
- Infra had multiple primary devices online simultaneously during the test while Desktop Commander had only the Oracle device online.

Observed footprint/resource snapshot on that host:

| Item | Approximate observed value |
|---|---:|
| Infra prototype repository/runtime footprint | ~29 MB |
| Desktop Commander installed footprint | ~233 MB |
| Infra Node agent RSS | ~63 MB |
| Infra Node gateway RSS | ~98 MB |
| Desktop Commander wrapper RSS | ~137 MB |
| Desktop Commander main server RSS | ~273 MB |

These are **reference observations, not universal claims**. Re-run measurements on the same host when v1 is ready.

## Must-win matrix

Infra should not be considered a full replacement until it wins or clearly ties every important category below.

| Category | Requirement |
|---|---|
| Install simplicity | Infra wins |
| No runtime dependency burden | Infra wins |
| Disk footprint | Infra wins materially |
| Idle RAM | Infra wins materially |
| Startup speed | Infra wins or ties |
| Reboot persistence | Infra wins or ties |
| Sleep/wake reconnect | Infra wins |
| Network-change reconnect | Infra wins |
| Multi-device availability | Infra wins |
| One-shot shell latency | Infra wins or ties |
| Interactive PTY | Infra wins |
| Persistent sessions | Infra wins |
| Long-running jobs | Infra wins |
| File read/edit | Infra wins or ties |
| Fast content/file search | Infra wins or ties |
| Binary transfer | Infra wins |
| Large-output handling | Infra wins |
| Process control | Infra wins or ties |
| Network/port inspection | Infra wins through structured tools |
| Docker/container operations | Infra wins through structured tools |
| Git common workflows | Infra wins through structured tools |
| macOS support | Infra wins |
| Windows support | Infra wins |
| Linux support | Infra wins |
| GUI automation | Infra wins before final cutover if GUI control is a required workflow |
| Elevated operations | Infra has safer explicit helper model |
| Auditability | Infra wins |
| Update + rollback | Infra wins |
| MCP tool ergonomics | Infra wins / fewer calls |

## Test scenarios

### Connectivity

1. reboot each device and measure time until gateway reports online.
2. sleep/wake Mac and verify connection recovers without manual restart.
3. switch Wi-Fi/network and verify stale socket is superseded.
4. disconnect gateway briefly and ensure agents reconnect.

### Shell/terminal

- `echo`/small command latency.
- 10 MB output command without loss/deadlock.
- interactive Python/Node shell.
- Codex CLI session maintained for at least 30 minutes.
- resize terminal.
- detach/re-attach.
- cancel process tree.

### Files/search

- read 1 KB, 1 MB, and large text files by range.
- exact edit and patch.
- search a repository containing `node_modules`/`.git` exclusions.
- search filenames and content with 10k+ files.
- watch/tail a growing log.

### Transfers

- upload/download 10 MB, 100 MB, and larger test artifacts.
- interrupt halfway, reconnect, resume.
- verify hash.
- ensure terminal remains responsive during transfer.

### System

- identify top disk consumers.
- identify process owning a port.
- list/restart a test container.
- fetch bounded logs.
- Git status/diff/log.

### Resource measurements

Capture:

- installed bytes
- idle RSS
- idle CPU over several minutes
- reconnect CPU spike
- bytes transmitted while idle
- gateway memory with 1, 3, and more connected devices

## Model-efficiency benchmark

Raw performance is not enough. Run equivalent natural-language tasks through ChatGPT and count tool calls.

Representative tasks:

- find port owner and stop/restart the right process.
- find a config value, patch it, and verify.
- inspect Docker failure and retrieve relevant logs.
- start an interactive dev server and read later output.

Infra should require fewer or equal calls while remaining understandable and auditable.

## Automated safe acceptance

For an already-enrolled device, `scripts/cutover-acceptance.py` runs the non-destructive
replacement-critical matrix in one pass: online identity, exec, file write/read/edit/hash,
search, process listing, system/network inspection, PTY, durable jobs, resumable binary
transfer, and (when advertised) desktop permission/discovery/clipboard/screenshot checks.
It mutates only a unique temporary directory and restores the clipboard.

```bash
export INFRA_ACCEPT_GATEWAY_HTTP=http://<private-gateway>:<port>
export INFRA_ACCEPT_DEVICE_ID=<device-id>
export INFRA_ACCEPT_ADMIN_TOKEN=<private-admin-token>
export INFRA_ACCEPT_MCP_HOST=localhost
python3 scripts/cutover-acceptance.py
```

Set `INFRA_REQUIRE_DESKTOP=1` for the final workstation cutover gate. Set
`INFRA_ACCEPT_INTERACTIVE_DESKTOP=1` only when it is safe for the runner to send a harmless
Escape key after desktop permissions are ready. `INFRA_SKIP_DESKTOP=1` is intended only for
staging older agents while a native artifact is being updated; it is not valid for final
workstation cutover.

Reboot, sleep/wake and network-switch tests remain deliberately outside this script because
they can interrupt the control channel and require explicit real-device scheduling.

## Reliability burn-in

Before final removal of Desktop Commander:

- run Infra as primary for normal workflows.
- record disconnects, stuck sessions, failed updates, and manual-repair events.
- do not cut over if reconnect/startup still requires routine manual intervention.

## Final cutover

Desktop Commander can be removed when:

1. all primary devices are reliably supported.
2. all routinely used Desktop Commander capabilities have an equal or better Infra path.
3. no must-win benchmark remains red.
4. v1 has survived a real-world burn-in period with a known rollback path.
