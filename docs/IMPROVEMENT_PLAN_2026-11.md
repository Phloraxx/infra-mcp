# Infra improvement plan — implementation handoff (14 November 2026)

This is a self-contained work order for a coding agent taking over the Infra v1 codebase. It records what to build, why, where the code is today, and how to prove each step finished. Read it together with the canonical context docs; nothing here overrides them:

- docs/ARCHITECTURE.md — system shape, transport, identity, failure semantics
- docs/CURRENT_REALITY.md — what is actually deployed and what is deliberately excluded
- docs/NEXT_STEPS.md — the owner decision record this plan extends (P0-A..P2 ladder)
- docs/PROTOCOL.md, docs/SECURITY_MODEL.md, docs/RELEASE_SIGNING.md, docs/RELEASE_PUBLISHER_KEY.md, docs/DASHBOARD.md, docs/CONNECTION_READINESS.md, docs/RELIABILITY_UX_TESTS.md, docs/BENCHMARK.md, docs/SESSION_PERSISTENCE.md

## 0. Standing rules for this repository

- **No pull requests.** Work happens on main or short-lived local branches merged to main by commit. Commit identity: user.name `Sourav P Bijoy`, user.email `71513365+Phloraxx@users.noreply.github.com` (use `git -c user.name=... -c user.email=...` per commit).
- **QA gate before any push touching the dashboard or gateway:** `node scripts/test-dashboard-browser.cjs` must print `Dashboard browser gate PASS:`. On the oracle VM it runs with `NODE_PATH=/tmp/qa/node_modules` and `INFRA_CHROMIUM_EXECUTABLE` pointing at the headless shell. Long runs go detached via nohup to a log file, then poll the log.
- **Deploy gotchas (learned the hard way, 14 Nov 2026):** `touch crates/gateway/build.rs` before any release rebuild or healthz keeps the old `+<sha>` stamp; copy new binaries to `<name>.new` then atomic `mv` (cp over a running binary fails with ETXTBSY); a gateway restart drops every agent link for roughly 1-5 minutes and browser event streams with them. Deploy target: systemd user unit `infra-v1-remote-burnin-gateway.service`, binaries in `~/.local/share/infra-v1-remote-burnin/bin/` named `infra-gateway-<short-sha>`. After restart verify `healthz` version, served `styles.css`, and agent reconnection in the journal, then add a row to the DASHBOARD.md deployment audit table with the binary sha256.
- **Design language:** the dashboard keeps the approved v3 console aesthetic; plain browser JS, no frameworks, no AI-generated look. Compact style with per-feature sections in app.js is fine; do not rewrite to React.

## 1. System snapshot (verified 14 Nov 2026)

- Repo head at time of writing: `564a13f` (docs). Deployed gateway: `0.1.1-rc.4+60bf1d9` behind the Cloudflare tunnel, bound tailscale-only at 100.79.230.22:18790.
- Recently landed: heartbeat deadline now 90s default, overridable with `INFRA_HEARTBEAT_TIMEOUT_SECS` (floor 30s), in `crates/gateway/src/agent_ws.rs` (`agent_heartbeat_timeout()`); dashboard v4.2 (fleet strip in the Connected card, per-row drop context, arrow-key row navigation, live activity count); disconnect retirement remains generation-guarded (verified in `agent_ws.rs` disconnect path before any store write).
- Fleet: 4 agents (Linux oracle-vm, macOS, Windows, Android). Mac previously hit `heartbeat_timeout` every 30-60 min; stalls under the new 90s deadline are now tolerated by design, and the remaining Mac question needs measurement, not more deadline tuning.

Key code anchors discovered during research (line numbers approximate; re-verify before editing):

| Area | Location | Fact |
| --- | --- | --- |
| Wire protocol | crates/protocol/src/lib.rs | `PROTOCOL_VERSION: u16 = 1`; capability strings already exist (`files.list_page`, `connection.readiness_v1`); `DeviceHello.capabilities: Vec<String>` plus a `metadata` map; `HelloAck` carries `control_frame_limit` and `reconcile_requested` |
| Dashboard embed | crates/gateway/src/dashboard_http.rs:22-26 | `include_str!` of index.html/app.js/styles.css — every UI change needs a binary rebuild |
| Transfer chunks | crates/agent/src/transfer.rs:15-16 | `DEFAULT_CHUNK = MAX_CHUNK = 256 KiB`; chunks travel as base64 inside JSON |
| Copy relay | crates/gateway/src/copy.rs:6-10 | 256 KiB chunks, 16 MiB default / 64 MiB max budget per call, 100 s cap, 20 s RPC timeout |
| Agent loop | crates/agent/src/main.rs (~197-219, ~444-545) | backoff 1s doubling to a 30s cap, no jitter, reset to 1s on clean close; receive loop never awaits worker permits; `agent_busy` on semaphore exhaustion; reconcile sent after every op and every ping |
| Gateway store | crates/gateway/src/store.rs | SQLite, migrations via `PRAGMA user_version` (currently >= 4); tables: devices, enrollments, pair_requests, audit_events; `audit_events` has no retention |
| Release verify | crates/cli/src/release_verify.rs + update.rs | Ed25519 strict manifest verify, anti-downgrade `authorize_update_version`, binary/version pinning all implemented; **not provisioned**: no owner key, producer not wired, bootstrap checksum-only |
| Existing tests | crates/gateway/src/tests.rs | 17 tokio tests incl. generation supersession, nonce replay, abort-after-dispatch no-replay, full-queue expiry, socket-loss returning outcome_unknown, timed-out op not retiring a healthy agent, pagination capability gating, signed auth + revocation |
| CI | .github/workflows/ | rust.yml (Rust), ci.yml (legacy TS), android-agent.yml, release.yml, windows checks; no clippy gate, no node --check, no cross-compile targets |

## 2. Workstreams

### W0 — Quick wins bundle (no protocol change, no agent rebuilds)

**W0.1 Serve the dashboard from disk, embedded fallback.** Problem: the dashboard is compiled in, so a CSS tweak forces a binary rebuild and a gateway restart that drops every agent link (this caused two user-visible outages on 14 Nov). Spec: at startup, resolve optional env `INFRA_DASHBOARD_DIR`; if set and the file exists, serve index/app.js/styles.css/vendor from it with an ETag (content hash) and `Cache-Control: no-cache`; otherwise serve the embedded bytes exactly as today. Default behavior with no env is byte-identical to current. A broken or missing directory must silently fall back to embedded. Acceptance: unit tests for override resolution and fallback; QA gate green; deploy a dashboard change with no binary rebuild and no service restart; healthz version unchanged. Files: crates/gateway/src/dashboard_http.rs, crates/gateway/src/main.rs.

**W0.2 Zero-console-error assertion in the browser gate.** Collect `page.on(pageerror)` and console errors from first load and assert zero after each major phase (sign-in, overview, device drawer, session pop-up, files, settings) in scripts/test-dashboard-browser.cjs. This would have caught the shipped `DOT is not defined` bug (a render-path ReferenceError that surfaced to the user as a persistent connection-lost banner) regardless of fixture shape. Keep the existing hostile-name/XSS fixture; do not weaken any existing assert.

**W0.3 Real-shape fixture snapshot.** Add a small script that fetches an authenticated `/api/overview` from a staging gateway, redacts device ids, names and uuids into QA-safe values, and writes `scripts/fixtures/overview-snapshot.json`. The browser gate then runs a second pass against this real-shape fixture so schema drift between the gateway and the QA fixture is caught at gate time. The 14 Nov bug shipped precisely because the fixture lacked a field production had.

**W0.4 CI hardening (.github/workflows/rust.yml).** (a) `cargo clippy --workspace --all-targets -- -D warnings`, with an initial baseline cleanup commit; (b) `node --check` on apps/dashboard/app.js and the QA script; (c) cross-compile smoke: `cargo check --target aarch64-apple-darwin` and `--target x86_64-pc-windows-gnu` for crates/agent and crates/cli (add targets via rustup in the job). There is no local macOS toolchain right now; this keeps the darwin agent target compiling while that is true.

**W0.5 Reconnect telemetry.** In store.rs, when a device reconnects and a prior disconnect was recorded within 24h, write an audit event `device.recovered` whose outcome carries the seconds between `last_disconnect_at_ms` and now. No new table. This plus the dashboard drop notes gives the 7-day soak (W8) its evidence stream. Also log the elapsed retirement time in the existing heartbeat-timeout warn line.

### W1 — Binary frames for bulk payloads

Problem: everything is JSON text frames on one WebSocket; file chunks are base64-in-JSON (+33 percent wire cost plus serde overhead); terminal write data rides as a String field; a control frame limit of 8 MiB is serialized as text. First benchmark of the copy path will hit serialization, not bandwidth.

Design: additive, capability-gated binary frames for payload-bearing operations only; control stays JSON.
- New capability strings `transfer.binary_v1` and `terminal.binary_v1`, advertised in `DeviceHello.capabilities`, consistent with the existing `files.list_page` / `connection.readiness_v1` pattern. `PROTOCOL_VERSION` stays 1; old agents keep the JSON path unchanged.
- Frame layout: 16-byte fixed header + raw payload. Magic `I` (u8), version (u8 = 1), kind (u8: 0 upload_chunk, 1 download_chunk, 2 terminal_data, 3 reserved), flags (u8), id as 16 raw UUID bytes (transfer or terminal id), offset u64 big-endian, length u32 big-endian, then `length` payload bytes. No JSON inside bulk frames.
- Agent side (crates/agent/src/transfer.rs, terminal.rs): handle `Message::Binary` only when the capability was negotiated; unknown binary on an un-negotiated connection is a protocol error and closes the socket. The receive loop in main.rs branches before deserialization.
- Gateway side (agent_ws.rs, transfer tooling, copy.rs): decode bulk frames, keep the JSON path for un-negotiated peers; the copy relay streams raw chunks end to end with no base64. Keep 256 KiB chunks initially; consider 512 KiB/1 MiB only after the W-benchmark.

Tests: protocol roundtrip unit tests (header encode/decode, offsets); gateway test with a fake binary-capable agent; copy integration test on both paths; benchmark before/after using the docs/BENCHMARK.md harness (1/16/100 MiB copy, p50/p95). Acceptance: 100 MiB copy through the gateway at >= 1.5x the base64 path throughput at equal chunk size, and no control-latency regression (ping round trip during an active transfer stays within 2x idle). Risk note: RFC 6455 guarantees ordering of text and binary frames on one socket, so interleaving is safe; the risk is receive-loop branch mistakes, covered by the W3 harness.

### W2 — Durable cross-device copy queue

Problem: MCP copy is a synchronous choreography capped at 16 MiB per call (64 MiB max, 100 s), so a 1 GiB file means dozens of manual calls (docs P1-D). Design: a SQLite-backed queue inside the gateway, reusing the existing two-transfer-id resume already implemented in copy.rs.

- New table `copy_jobs` (next `PRAGMA user_version` migration; current version is 4, verify in store.rs before writing): id, created_at_ms, source_device, dest_device, source_path, dest_path, state (queued/running/paused/failed/completed/canceled), bytes_done, total_bytes, chunk_offset, sha256 progress fields, source_transfer_id, dest_transfer_id, last_error.
- One gateway-internal tokio task per running job drives the same transfer actions the synchronous copy uses today; on restart the task list is rebuilt from the table and resumes at the stored offset. Either endpoint offline pauses the job; both online resumes. State transitions must be crash-safe (write-ahead row updates before acting).
- MCP surface: `copy.start` (returns job id), `copy.status`, `copy.cancel`. The existing single-call copy remains the fast path for small files with unchanged semantics.
- Retention: completed rows deleted after 7 days, failed after 30, via the W6 cleanup pass.
- Acceptance (from docs): a 1 GiB copy starts with one call plus bounded status polling; digest verified after a forced mid-stream disconnect and gateway restart. Tests: state-machine unit tests; W3 scenario (kill mid-copy, restart gateway, resume, digest match; offline pause/resume).

### W3 — End-to-end fault-injection harness

Problem: crates/gateway/src/tests.rs is genuinely strong, but everything is in-process with mocked agents. Nothing runs a real agent binary against a real gateway over real sockets with failures injected between them. The whole design leans on failure semantics (outcome_unknown, supersession, resume, reconcile) being correct in the composed system.

Design: `tests/e2e/` (new integration target, `#[ignore]`-free and CI-wired, or a workspace member with a feature gate):
- Harness spawns the gateway binary on a loopback port with a temp SQLite dir and generated credentials, and the agent binary with a temp state dir pointed at the loopback WS URL.
- A small tokio TCP proxy between agent and gateway with injectable faults: cut connection, delay frames N seconds, split writes, blackhole direction, stall pongs past the heartbeat deadline.
- Scenarios, mapped to the failure matrix in docs/NEXT_STEPS.md: (1) exec dispatched then socket cut before response returns outcome_unknown with no replay after reconnect; (2) download cut mid-chunk resumes from offset with identical sha256; (3) copy cut mid-stream resumes after gateway restart (needs W2); (4) terminal open/write, cut, reconnect, read from offset: retained range has no lost or duplicated bytes; (5) stalled pong past deadline retires the generation and a newer connection is never clobbered by the late retire; (6) double connect: new generation wins, old socket close does not flip the device offline; (7) gateway restart mid-job: job survives, agent reconciles.
- Assert observable state (store rows, audit events, response values), not just absence of panic. Wire as a separate CI job in rust.yml, budget under 10 minutes. This harness is the sign-off gate for W1 and W2; if sequencing allows, build it first.

### W4 — Signed release provisioning (code exists; operations do not)

crates/cli/src/release_verify.rs and update.rs already implement strict Ed25519 manifest verification, anti-downgrade authorization and binary/version pinning, with unit tests. What is missing is entirely operational (docs/RELEASE_SIGNING.md, docs/RELEASE_PUBLISHER_KEY.md):
1. Owner generates the Ed25519 publisher key offline; the public key is embedded at compile time; the private key lives only as a GitHub Actions secret. The repo is public: keys, tokens and gateway URLs never enter the repo or logs.
2. release.yml gains a producer job that signs the manifest (assets + versions + sha256) and a verification job that runs the CLI verifier against the published artifacts with a CI test key first, production key after.
3. Publish a signed installer manifest so the one-command bootstrap verifies before exec; keep the checksum-only path documented as unsafe and temporary.
4. Anti-rollback: `authorize_update_version` is present; add a release-index floor check so an old-but-valid signed release cannot be served as current.
Acceptance: full cycle in CI with a test key (old-to-new updates, downgrade refused, tampered manifest refused), then one real signed update on a Linux agent. Every later agent-side change (W1, W7) ships through this path.

### W5 — Reconcile chatter reduction

The agent sends a full ReconcileSnapshot after every operation and every ping, and the gateway emits `sessions_changed` on each, driving SSE refresh churn. Design: add a content hash over the snapshot items (id, kind, status, boot_id, started_at). Agent keeps the last hash it sent and sends a compact `ReconcileUnchanged` (or an empty snapshot plus hash field) when nothing changed; gateway compares per-device and only emits on change. Capability string `reconcile.hash_v1`; old agents unaffected. Tests: protocol unit test; gateway test asserting no sessions_changed when the hash matches; W3 assertion that one reconnect produces exactly one changed-reconcile. Acceptance: visible reduction of reconcile log noise at idle with 4 connected devices.

### W6 — Audit retention

audit_events (store.rs schema) grows unbounded on a box with 74 percent disk. Add a cleanup pass in the gateway: a tokio interval (daily) deleting rows older than `INFRA_AUDIT_RETENTION_DAYS` (default 180), using the existing at_ms index; reuse the same pass for expired copy_jobs rows from W2. Test: seed old and recent rows, run the pass, assert deletion and retention.

### W7 — Agent reconnect jitter (gated on darwin toolchain)

agent/src/main.rs backoff is exponential 1s to 30s with no jitter and a hard reset to 1s on clean close; a gateway restart makes the whole fleet reconnect in a synchronized wave (observed 14 Nov). Add plus-or-minus 25 percent jitter to each sleep, keep the 30s cap and reset semantics. This is a tiny patch but requires per-platform agent rebuilds; macOS needs a toolchain that does not currently exist locally. Ship it through the W4 signed update path. The W0.4 cross-compile job keeps the target healthy in the meantime.

### W8 — Soak and telemetry program

Run the docs/NEXT_STEPS.md failure matrix twice: fast fault cases inside the W3 harness, and a 7-day live-fleet soak (sleep/wake, network switch, edge restart, idle periods) observed through W0.5 recovery events and the dashboard drop notes. Deliverable: numbers appended to docs/BENCHMARK.md and docs/STABILIZATION_STATUS.md — reconnect p95, drops per device by reason, outcome_unknown counts, duplicate side effects (expected zero). This closes the Mac heartbeat investigation with evidence instead of hypothesis.

## 3. Sequencing

1. **Phase 1, no fleet coordination:** W0 (all), W3 harness, W6, W5. Everything here is gateway/CI/test-side and deployable independently.
2. **Phase 2, protocol-additive:** W1 binary frames, behind capability negotiation, old fleet unaffected.
3. **Phase 3, operations:** W4 signing provisioned first (it is the delivery path for agent changes), then W2 durable copy, then W7 jitter.
4. **Phase 4:** W8 soak, update the docs, and only then revisit transport choices if benchmarks demand it.

Every phase ships behind green CI plus a green browser gate, and a one-device staged rollout before fleet-wide agent updates.

## 4. Non-goals (unchanged from docs/CURRENT_REALITY.md)

No Redis/Postgres/message queues; no QUIC/WebTransport; no second gateway process or dashboard backend; no React/Electron; no privileged helper until a concrete owner workflow needs it; never auto-retry outcome_unknown; no public raw agent port; do not break the current MCP client compatibility.

## 5. Definition of done for the whole effort

- CI green with clippy deny-warnings, cross-compile checks, the W3 e2e job, and the browser gate with the zero-console-error assert.
- Copy/read benchmark deltas recorded before and after W1/W2 (docs/BENCHMARK.md).
- A real signed update cycle demonstrated on a real device (W4).
- 7-day soak report with the failure matrix results (W8), appended to docs/STABILIZATION_STATUS.md.

## 6. Known traps for the implementing agent

- Do not trust line numbers in this file blindly; re-locate anchors with grep before editing.
- The QA fixture on purpose uses hostile names and missing optional fields; never fix the gate by weakening the fixture. Add coverage, never remove it.
- Gateway restarts are user-visible: schedule restarts detached, expect 1-5 min of agent reconnect, and update the DASHBOARD.md audit table with real sha256 values after each deploy.
- The build.rs version stamp only refreshes when its inputs change; touch it before release builds.
- `cp` over a running binary fails with ETXTBSY; always build to a temp name and `mv`.
