# Hermes bridge: one MCP endpoint for agent, memory and devices

Status: experimental and opt-in. No production deployment without acceptance tests.

## Goal and architecture

Infra is the independent device control plane; Hermes is the personal AI agent and owner of its built-in memory. An external ChatGPT/Mistral/Codex MCP client connects to one authenticated Infra endpoint.

ChatGPT / Mistral / Claude / Codex
  |
  HTTPS / OAuth (Infra MCP)
  |
  Infra gateway
  |-- existing device RPC -> Oracle, Mac, Windows, Android
  |-- optional Hermes bridge -> local authenticated Hermes Runs/Sessions API
  |-- optional Hermes memory bridge -> Hermes MemoryStore and state.db

No extra database, no separate model invocation for a simple memory read, and no Hermes fork.

## Tools

The new hermes tool uses:
- capabilities: GET /v1/capabilities, no inference
- start: POST /v1/runs, requires input and stable client-generated request_id
- status and stop: GET/POST /v1/runs/{run_id}(/stop)
- sessions: paginated GET /api/sessions
- messages: GET /api/sessions/{session_id}/messages
- search: Hermes native read-only session_search across its own state.db

Start is idempotent when the same request_id is retried. It does not block until the model run completes. Poll status, optionally stop. Avoid starting again with a different request_id after uncertain outcomes.

The new hermes_memory tool supports read, add, patch and remove with target user or memory. Mutations use Hermes MemoryStore, which supplies its own locking, entry budgets, atomic writes and memory-content validation. No raw append, direct editing or SQLite database modifications are used. Read before patching/removing. Writes are disabled unless explicitly enabled by the operator.

## Opt-in gateway environment

All values here are examples; substitute paths for your host and keep real credentials out of Git.

    INFRA_HERMES_ENABLED=1
    INFRA_HERMES_BASE_URL=http://127.0.0.1:8642/
    INFRA_HERMES_API_KEY_FILE=/run/user/1000/infra-hermes-api.key
    INFRA_HERMES_HOME=/home/example/.hermes
    INFRA_HERMES_SOURCE=/home/example/.hermes/hermes-agent
    INFRA_HERMES_PYTHON=/home/example/.hermes/installs/ACTIVE-ENV/venv/bin/python
    INFRA_HERMES_MEMORY_WRITES=0

The key file contains ONLY the Hermes bearer secret; chmod 0600 and restrict it to the Infra gateway owner. Do not pass the token in CLI arguments or commit it. Hermes HTTP remains on loopback. This bridge refuses non-loopback base URLs. Use the active Hermes dependency venv for Python, not the bare embedded python binary. Hermes updates can change this path.

Without INFRA_HERMES_ENABLED=1 and valid configuration the tools fail closed. Memory writes require a separate INFRA_HERMES_MEMORY_WRITES=1 setting. Do not enable it until safe read-only operation and authorization are verified.

Note: existing Infra MCP clients presently share the owner authorization level. This bridge does not implement fine-grained OAuth scopes. A future stage must enforce separate agent/memory/device permissions server-side for less-trusted clients.

## Prevent recursive Hermes delegation

Oracle Hermes already uses Infra as an MCP server. Before deploying, configure Hermes's Infra MCP tool selection to include device tools only: devices, exec, terminal, files, jobs, system, network, processes, services, containers, git, desktop, transfer, copy. Exclude hermes and hermes_memory from the tools Hermes loads from Infra; otherwise the agent could call back into itself.

This Hermes-side tool selection is defense against accidental looping, NOT a security boundary. Add gateway-side client identity/scope checks before providing broader remote delegation access to other users.

## Acceptance gates

1. Existing Infra device tools are unchanged, including when Hermes is unavailable.
2. A staging gateway can authenticate to the local Hermes capabilities API; wrong key or inaccessible API must fail safely.
3. Read-only memory operations return the intended Hermes profile data; other profiles cannot be selected by untrusted tool arguments.
4. In a disposable HERMES_HOME, read -> add -> patch -> remove uses Hermes MemoryStore successfully.
5. One non-destructive approved test run returns a run_id, reaches a terminal status and respects Hermes tool approvals. Repeating start with the same request_id must not create a second run.
6. Session search gives meaningful results from the intended profile. Error cases, oversized outputs, malformed IDs and concurrent writes are tested.
7. Test Mistral and eligible ChatGPT MCP clients, using minimally scoped credentials.
8. Stage/canary first, independent approval for production rollout, and verify rollback.

## Not included in this first change

- Automatic capture of every ChatGPT/Mistral turn (client hooks or MCP calls would be required).
- Semantic/vector memory provider, Honcho, or PostgreSQL.
- Automatic multi-profile memory merging (memory remains Hermes profile-scoped).
- Changes to Hermes source, Infra native device agents, or production services.
