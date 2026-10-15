# MCP tool model

Infra should expose a compact, predictable MCP surface. Tool design is optimized for model selection and low call count rather than mirroring every OS command.

The names below are conceptual; exact MCP schema details may evolve.

## 1. `devices`

Actions:

- list
- status
- capabilities
- rename
- revoke (admin/consequential)

Return stable device ID, friendly name, platform, architecture, online state, version, capabilities, startup mode, last seen, and active connection metadata.

## 2. `exec`

One-shot user-level command execution.

Inputs:

- device
- command
- cwd
- shell (`auto`, platform shell, or explicit supported shell)
- environment overrides
- timeout
- output limits
- optional `background`

For interactive programs use `terminal`, not `exec`.

If output exceeds the inline limit, return a durable/spooled result reference or offset token rather than silently losing the beginning/end.

## 3. `terminal`

Actions:

- open
- write
- read
- resize
- signal
- status
- attach
- detach
- close
- list

This is the preferred surface for Codex CLIs, REPLs, SSH, database consoles, interactive installers, and dev servers that need an attached shell.

## 4. `files`

Actions:

- read by line range or byte range
- read_many
- write/atomic_replace
- append
- exact_edit
- patch
- list
- tree
- stat
- mkdir
- copy
- move
- trash
- delete
- hash
- tail
- watch

Text reads return explicit truncation/pagination information.

`trash` is recoverable where the platform supports it. `delete` is permanent and should be classified as destructive.

## 5. `search`

Actions/modes:

- filename/glob
- content/grep
- regex or literal
- optional file pattern
- max results
- context lines
- include/exclude hidden/ignored directories
- continuation cursor/session for long searches

Implementation should use efficient native search strategies. Do not recursively read every small file into application memory as the primary design.

## 6. `processes`

Actions:

- list
- find
- inspect
- signal
- terminate
- tree

Return normalized PID, parent PID, executable/name, command, user, state, CPU/memory when available, start time, and working directory when obtainable without excessive cost.

## 7. `system`

Actions may include:

- summary
- uptime
- CPU/load
- memory
- disks/filesystems
- top disk usage
- OS/runtime details
- environment/capability checks
- reboot/shutdown only through explicit privileged path

Common diagnostics should be structured rather than requiring `df`, `du`, `free`, `wmic`, and platform-specific pipelines from the model.

## 8. `services`

Normalize service management across systemd, launchd where practical, and Windows SCM.

Actions:

- list/find/status
- logs where supported
- start/stop/restart (possibly privileged)
- enable/disable (privileged)

## 9. `network`

Actions:

- listeners
- connections
- port_owner
- resolve
- ping/test_connectivity
- HTTP health probe
- interface summary

A prompt such as “what owns port 3000?” should be one structured call.

## 10. `containers`

Docker/compatible container support when available:

- list
- inspect
- logs with cursor/tail
- stats
- start/stop/restart
- exec (may route through terminal semantics)
- remove (destructive)
- images
- compose/project operations where useful

Container support is optional per-device capability.

## 11. `git`

Common structured actions:

- status
- diff
- log
- branches
- remotes
- pull/fetch
- add/commit
- push

The module should not attempt to replace the Git CLI. Complex Git workflows can use `exec`/`terminal`.

## 12. `transfer`

Cross-gateway/device binary movement:

- upload to device
- download from device
- device-to-device copy via gateway relay or direct optimization later
- status
- resume
- cancel

Return progress, bytes transferred, total bytes, throughput, and verified hash.

## 13. `desktop`

Later-phase actions:

- displays
- windows/apps
- screenshot
- accessibility/UI tree
- find element
- focus
- click semantic element
- type text
- key/chord
- scroll
- mouse fallback
- clipboard get/set
- wait_for condition

Prefer semantic target selectors such as application/window/role/name over raw coordinates.

## Device selection

All device-bound calls accept a stable device selector. Friendly names may be resolved by the gateway, but internal routing should use stable IDs.

If a selector is ambiguous, fail clearly. Do not guess between two similarly named online machines.

## Safety metadata

Each action should carry a classification such as:

- read-only
- reversible write
- consequential write
- destructive
- privileged

The MCP annotations/presentation should reflect this accurately.

## Output design

Tools should optimize for model consumption:

- structured fields before prose
- bounded output
- explicit `truncated`, `has_more`, `next_offset`, or cursor fields
- stable IDs for follow-up operations
- separate stdout/stderr where useful
- normalized errors
- timestamps only where they add value

Avoid returning huge raw process lists, terminal histories, or file contents when a filtered request can answer the question.

## Current implemented subset — 2026-10-15

Available through the Rust MCP gateway now: `devices`, `exec`, `files`, `search`, `processes`, `jobs`, `terminal`, and `transfer`.

- `files`: byte-range UTF-8 read, atomic-style write, append, exact edit with expected replacement count, SHA-256 hash, list, stat, mkdir, copy, move, explicit permanent delete; recoverable `trash` currently reports unsupported.
- `search`: filename or content mode, regex/literal, case control, hidden-file control, git-ignore-aware walking, glob file filtering, bounded results and bounded context.
- `jobs`: start/list/read/cancel with worker process and byte-offset spool.
- `terminal`: open/list/read/write/resize/close using persistent PTY/ConPTY workers.
- `transfer`: persistent upload/download begin/chunk/commit/status/list/cancel, explicit byte offsets, idempotent upload retries, bounded base64 chunks, staging files and SHA-256 verification. Device-to-device relay is orchestration on top of the two device endpoints and is not optimized yet.

Still targeted: line-range reads, unified patch, tail/watch, richer process inspection/signals, structured system/service/network/container/Git modules, semantic desktop control and privileged operations.
