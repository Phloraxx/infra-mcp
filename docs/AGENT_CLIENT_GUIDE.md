# Infra MCP: agent integration contract

Status: source-branch implementation only. **Not deployed** to the live gateway or to personal devices. The authoritative contracts are the gateway's live MCP tool list and each device's advertised capabilities.

## Purpose and layering

Infra is an authenticated multi-device execution control plane, not a competing AI agent, memory store or coding sandbox. ChatGPT, Hermes, Pi and other clients should do reasoning; Infra routes permitted file, process, terminal, Git, device and OS operations. Keep the small set of typed tool families rather than creating dozens of one-command wrappers.

## Agent workflow

1. Discover using **devices** with **compact=true**. Select an online, capability-compatible device; use its stable ID if names overlap. Omit compact to obtain full metadata, including connection diagnostics.
2. Prefer typed operations over shell: **system**, **git**, **search**, **files**. Use **exec** for short noninteractive commands, **terminal** for live sessions, and **jobs** for unattended long work. Android's capabilities are restricted compared with desktop.
3. For **files**, **jobs**, and **terminal**, inspect the exported action-specific **oneOf** schema; each action advertises required fields. JSON arguments remain in their previous flat wire format.
4. Minimize output: bound reads and searches, preserve returned IDs and **next_offset**. Use **jobs.list** with **compact=true** for status-only lists rather than dumping command strings.
5. Inspect status, error_code, retryable and next_action. Never automatically reissue a mutating operation after **outcome_unknown**.

## Illustrative MCP requests

Device discovery:

~~~json
{"name":"devices","arguments":{"compact":true}}
~~~

Read a bounded file:

~~~json
{"name":"files","arguments":{"device":"desktop","action":"read","path":"/home/me/repo/README.md","offset":0,"length":4096}}
~~~

Start a long-running command, preserving its returned UUID:

~~~json
{"name":"jobs","arguments":{"device":"desktop","action":"start","command":"cargo test","cwd":"/home/me/repo"}}
~~~

Wait for output from that *same* job:

~~~json
{"name":"jobs","arguments":{"device":"desktop","action":"wait","job_id":"<job-uuid>","offset":0,"length":4096,"wait_timeout_ms":10000}}
~~~

The new wait action returns the same job/output/offset fields as read, plus **wait_reason** (output, finished or timeout) and **waited_ms**. The wait timeout defaults to 10 seconds, capped at 20 seconds, and does NOT terminate the underlying job. Continue from next_offset; a timeout is a normal non-terminal observation. Do not re-run the start command.

The native agent advertises **jobs.wait_v1**. New gateways reject wait before dispatching to old agents that lack it. Use jobs.read for older device binaries. Wait happens in a blocking worker, not in the async heartbeat loop.

Brief status-only job list:

~~~json
{"name":"jobs","arguments":{"device":"desktop","action":"list","compact":true}}
~~~

## Compatibility and safety

- Compact mode is opt-in. Without compact=true, devices and jobs.list preserve the existing full response structure.
- Mutating actions remain subject to existing authorization and OS permission requirements; this work adds no privileged execution.
- Missing action-specific required fields should be rejected before device dispatch.
- Handle partial output and pagination; outputs can be truncated. Unknown outcome is NOT an invitation to retry.
- Do not commit machine identifiers, secrets, raw logs or system topology to public docs.

## Acceptance before rollout

Run: cargo fmt --all; cargo test -p infra-agent -p infra-gateway -p infra-protocol; python3 scripts/smoke-mcp-contract.py.

Validate actual ChatGPT/Hermes/Pi MCP clients with the oneOf schema, plus native macOS/Windows agent wait behavior and heartbeat liveness. Deploy gateway changes first; upgrade the native agents before using wait. Do not touch live devices/gateway from fixture tests or infer deployment from a passing build.
