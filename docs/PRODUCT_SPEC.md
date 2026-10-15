# Infra v1 product specification

## Problem

ChatGPT can be far more useful when it can directly inspect and operate the user's own computers, but existing remote-control MCP solutions tend to have one or more of these weaknesses:

- only one machine is reliably connected.
- remote agents are heavy and dependency-rich.
- terminal execution is request/response rather than persistent.
- large outputs and binary files are awkward.
- reconnect after sleep/reboot is unreliable.
- tool sets are fragmented, causing excessive model tool calls.
- GUI automation is coordinate-heavy rather than semantic.
- privilege models are either too restrictive or dangerously all-powerful.

Infra v1 should solve these as a coherent product.

## Primary user

A technical owner controlling personally authorized machines from ChatGPT. Typical machines include:

- ARM64 Linux server/VM
- ARM64 macOS laptop/desktop
- x64 Windows desktop/lab machine

The design should remain clean enough for broader use later, but personal-use efficiency comes first.

## Core jobs to be done

Infra should make prompts such as these routine:

- “check what is consuming disk on Oracle and clean the safe caches”
- “open my Mac project, run Codex, and keep the session alive”
- “find where this config is defined and patch it”
- “tell me what owns port 3000”
- “restart this container and verify health”
- “copy this build artifact from Windows to the server”
- “watch these logs while I reproduce the bug”
- “open the app, click the named control, and verify the result”

The model should not need many low-level calls to accomplish ordinary tasks.

## Must-have v1 capabilities

### Connectivity

- multiple devices under one gateway
- stable pairing and revocation
- automatic startup
- automatic reconnect with backoff
- stale-connection supersession
- useful offline/last-seen state

### Execution

- one-shot user-level command execution
- persistent interactive PTY/ConPTY sessions
- process listing/find/signal/kill
- durable/background jobs with incremental output

### Files

- read/write/append
- atomic replacement
- block edit/patch
- list/tree/stat
- fast filename/content search
- copy/move/mkdir
- trash/delete
- watch/tail
- binary transfer with progress, resume, and hashes

### Structured operations

- system health/disk/memory/load
- services
- network listeners/connections/port ownership
- Docker/container operations where available
- Git status/diff/log/branch/commit/pull/push basics

### Security and maintenance

- per-device identity
- protected credential storage
- durable audit metadata
- separate optional privileged helper
- signed/authenticated updates with rollback
- `status`, `doctor`, `logs`, `repair`, `update`, `uninstall` CLI flows

## Later v1.x capabilities

- semantic desktop automation
- screenshots and targeted screen regions
- clipboard
- multi-display support
- optional tray app
- QUIC/WebTransport transport
- richer document-aware file adapters if they provide value beyond normal local tooling

## Explicit non-goals for the first implementation

- building a high-frame-rate consumer remote-desktop streaming product
- replacing SSH for human terminal use
- enterprise MDM/RBAC/multi-tenant administration
- running an LLM on the device agent
- implementing every cloud provider or deployment platform in the core
- hiding all shell access behind narrowly allowlisted operations

## UX requirements

### Installation

The desired user experience is one command or one installer package, then pairing.

After installation:

- startup persistence is configured automatically.
- `infra status` explains whether the agent is connected.
- `infra doctor` diagnoses common failures.
- `infra reconnect` is simple and memorable.
- `infra repair` can restore a broken installation without destroying device identity unnecessarily.

### ChatGPT usage

If several devices are online, tools accept a `device` selector. If exactly one device is relevant/online, the gateway may allow omission based on deterministic rules.

Results should be concise and structured. Pagination/offsets should be used rather than truncating silently.

### Failure messages

Errors must answer:

1. what failed
2. whether anything may already have changed
3. whether retry is safe
4. what action can recover

## Performance direction

The benchmark is comparative, not purely synthetic. Infra must materially outperform the current heavy alternative on installation size and idle resource use while meeting or beating it on normal command latency.

Desired characteristics:

- agent starts quickly and idles quietly.
- no Node/npm runtime on clients.
- interactive output is streamed rather than buffered to command completion.
- large file transfer does not block terminal traffic at the protocol layer.
- search uses efficient native walking/grep strategies rather than loading every candidate file into memory.

See `BENCHMARK.md` for the actual acceptance matrix.

## Product success

Infra can replace Desktop Commander when:

- all regularly used machines stay connected as reliably or better.
- ordinary development/operations tasks require fewer or equal model tool calls.
- terminal/session behavior is clearly better.
- filesystem/search/transfer behavior is at least as capable.
- installation/reboot/repair workflows are simpler.
- resource use is substantially lower.
- the owner no longer needs Desktop Commander as a fallback for any normal workflow.
