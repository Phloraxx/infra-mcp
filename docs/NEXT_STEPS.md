# Infra v1 — next implementation plan
Decision draft: 23 October 2026. Read [CURRENT_REALITY.md](CURRENT_REALITY.md) first. **Implement incremental changes, not a replacement architecture.** This file replaces the ambiguous "future" portions of older roadmap documents as the short-term work order.

## Summary: what actually matters

**Priority order:** (1) reliably reachable without Tailscale, (2) authenticated public MCP and dependable reconnect, (3) one-command/one-tap installation and actionable recovery, (4) measured speed and efficient MCP tool results, (5) secure releases and replacement acceptance, (6) removal of legacy dead weight.

Do not treat an isolated successful `/healthz` request as proof of a working signed WSS connection, OAuth login, encrypted control, durable session, or clean fallback.

## 1. Public access without compulsory VPN — pick the simplest path

```text
ChatGPT ─ HTTPS OAuth/MCP ─┐
Owner browser ─ HTTPS ─────┼─ Public DNS + TLS ingress
Mac/Linux/Windows ─ WSS ──┤   (existing Cloudflare Tunnel first)
Android signed agent ─ WSS┘             │
                                      ▼
                          127.0.0.1:private-port
                             Rust infra-gateway
                               SQLite + MCP
                                      │
                            signed outbound sockets
```

**One stable domain, one gateway, one login/pairing model.** This design still needs a reachable host (Oracle or another server), DNS, and an HTTPS ingress. It removes Tailscale from the clients' critical path; it is not magic peer-to-peer networking.

### Candidate ingress paths

| Option | Advantages | Risks and cost | Decision |
| --- | --- | --- | --- |
| Existing Cloudflare Tunnel -> Rust **loopback** | Already running; no new public inbound port; websocket supported; TLS edge; minimal infra changes | Additional edge latency, Cloudflare dependency, occasional edge-initiated WebSocket reconnect; edge can see traffic terminated there | **First staging experiment** |
| Existing public 443 reverse proxy -> Rust loopback | Fewer networking hops; provider-independent if DNS/TLS managed independently | Public ingress hardening and certificate/ports/other websites on shared Oracle host; more operator responsibility | **Benchmark-backed fallback** |
| Tailscale tailnet IP for everyone | Good private-network performance, simple ACL if all devices joined | VPN lifecycle on each device, inconsistent mobile experience, current Mac setup still requires tailnet | Keep **optional**, not default |
| New custom relay, broker, QUIC, mesh, headscale, WebRTC/P2P | Might help some specialized future use | New code, credentials, deployment, NAT, security, failure modes | **Reject for v1** |

Official references: https://developers.cloudflare.com/network/websockets/ and https://developers.cloudflare.com/cloudflare-one/faq/cloudflare-tunnels-faq/. Cloudflare may restart edge hosts and close active WebSockets: reconnect/backoff and generation supersession are required regardless of tunnel selection.

### Non-destructive migration sequence

1. Document the **real remote-managed Cloudflare origin route**: inspect its host, service target, routes `/agent`, `/mcp`, `/enroll` and all applicable Access/WAF rules. Do not guess it from DNS alone or dump tunnel tokens to Git/logs.
2. Create a **separate staging instance** bound only to loopback with an isolated SQLite DB, port, hostname and test identity. Do not point staging at the live registry or reuse enrollment secrets. No Tailscale required for the staging instance.
3. Route staging hostname through the existing outbound Cloudflare Tunnel to that loopback port. Test `GET /healthz` and unauthenticated 401 on restricted paths. Test authenticated MCP client discovery; ensure OAuth redirect URI/issuer/resource/PKCE/tokens actually work after proxy.
4. Enroll a **test Mac/Linux agent** using `wss://staging-host/agent` and verified TLS/certificate hostname. Test terminal, `exec`, files and reconnection. Test the Mac with its Tailscale client **disconnected only for the test**, never stop the Oracle daemon; confirm signed WSS works without VPN.
5. Repeat on phone with Wi-Fi then cellular without Tailscale; handle system background limits instead of promising always-online. Do not bypass runtime permission/consent or turn on restricted services.
6. Prepare a reversible `infra config set gateway wss://…` or `infra reconnect --gateway …` UX; back up only non-secret configuration. Preserve existing device identity and session history wherever compatible; test migration and rollback. Do not require manual JSON editing.
7. After staging acceptance, change canonical Rust gateway bind/ingress together under a maintenance plan with independent rollback. Keep the old tailnet endpoint until confirmed signed reconnect; avoid same-port dual-binding assumptions. Never remove an existing shared tunnel route blindly.
8. Test browser sign-in, ChatGPT MCP, enrollment, desktop controls, transfer, long-lived PTY, Internet/IPv4/IPv6/DNS failures, edge restart and 24h wake/reconnect before decommissioning old routes. One-step rollback: restore prior origin URL and device config.

**Security:** Cloudflare Access/service-token headers may interrupt ordinary WebSocket or ChatGPT OAuth flows; do not put an untested browser identity challenge before `/agent`. TLS and a signed agent challenge are necessary; signed identity does **not** provide additional end-to-end traffic confidentiality through the TLS-terminating edge. Rate-limit enrollment/auth; do not publish unprotected MCP. A public hostname must work with explicit `Host`/Origin validation; replace hard-coded private IP host allowlist carefully, not with a wildcard.

### P0 acceptance

- Mac + Linux direct WSS connect over public port 443 with VPN disabled and same device identity.
- Android reaches gateway on mobile data without Tailscale, within OS-authorized service state.
- Unauthenticated `/mcp` and dashboard APIs remain unauthorized; signed agent identity and invalid/revoked enrollment tests pass.
- A gateway/edge restart triggers a fresh generation, not stranded "online" state; PTY/job continuity is exactly what device worker contracts permit.
- Rollback and old services remain operable.

## 2. Reconnect, failures, observability

Instrument **without adding Prometheus/Redis/another DB**: structured, locally rotated logs with request ID, hashed/redacted device identifier, gateway generation, connection lifecycle, error class, operation class and elapsed time. Never log command bodies, file bytes, screenshots, tokens or private keys.

### Distinguish failure semantics

| Condition | Agent/dashboard behavior | What is never safe |
| --- | --- | --- |
| DNS/TLS/edge unavailable | Connecting/Retrying + bounded exponential backoff with jitter, last success and local `infra doctor` result | Reporting "connected"; disabling certificate checks |
| Device sleeping/unreachable | Last seen, possible sleep/wake evidence only if known, next step | Pretending the gateway can wake an arbitrary disconnected computer |
| Heartbeat timeout | Close stale generation and allow replacement; classify server and local logs | Infinite "online" state |
| Busy agent / constrained resources | Explicit `agent_busy` (no blocked control loop); queue only a safely bounded amount | Letting large jobs starve Pong/liveness |
| Mutating command dispatched, response lost | `outcome_unknown`, stable request ID and inspection guidance | Transparent automatic retry |
| Partial transfer | Persist IDs/offset/hash, explicit resume/status/cancel; show progress | Retrying an uncertain overwrite blindly |
| Revoked identity | Reject reconnection and guide deliberate re-pairing | Silent implicit re-enrollment |
| Android background restriction | Explain "needs foreground/user action" with OS-granted capabilities | Fake persistent service/root or background permission bypass |

Mac field observation to investigate: repeated gateway heartbeat timeouts and separate "connection reset without closing handshake" events; LaunchAgent/Keychain and local doctor have independently reported healthy at times. **These logs do not prove either Cloudflare or Tailscale is to blame.** Existing agent patch prioritizes Pong and prevents permit waits in the receive loop, but real soak testing is still required.

### Minimum failure-test matrix

- 30/60/120 min idle, repeated low-rate commands, burst (10–50 concurrent reads), stalled large output.
- Sleep/wake and Mac network switch; DNS loss/recovery; IPv4/IPv6; restart agent; restart gateway; Cloudflare edge interruption.
- Double simultaneous connections for one device; old socket still open; source agent disconnected mid-copy; destination reboots mid-copy.
- Android app foreground/background, lock/unlock, Doze, boot and reconnection after user action; access lease expiry and revoke.
- Every case must record elapsed reconnection, count of lost/unknown results, duplicate side effects, active session recovery and CPU/memory/battery impacts.

## 3. UX: less friction, fewer questions

### Install/pairing: one default flow

- Dashboard **Add device** chooses Desktop (macOS/Windows/Linux) or Android. One recommended copyable native install command for desktop; QR/deep link for Android enrollment after local approval.
- One-time code expires and is renewed safely; do not display unrelated transport/tunnel choices in first-run UX. It should report **Downloading -> Installing -> Authorizing -> Connected**, with a real agent handshake check before "done".
- Installer and CLI show one clear `infra status` / `infra doctor` result and "Copy diagnostics" with redaction. Admin credentials should not be manually pasted into .env.
- Support unique owner-selected display aliases without changing cryptographic device IDs. Avoid duplicate-name routing ambiguity.
- Default endpoint is `wss://<control-host>/agent`; VPN is an Advanced option. Existing client VPN should not be required to pair.
- On macOS permissions, explain exactly which Accessibility/Screen Recording permission is missing and where to grant it; never claim the agent can grant TCC itself.
- Windows UAC and Android OS permission/installer prompts remain explicit local-user decisions.

### Main dashboard: status should help act

Show concise **Ready**, **Reconnecting**, **Offline**, **Needs permission**, **Needs update**, **Restricted on Android** where signals truly establish them; don't infer "sleeping" from elapsed time alone. Device card: platform, last seen, version compatibility, active work, short reason, **Check connection**. Detail: "What works", recent failures, update/repair recommendation, last safe audit event. Recoverable local repair can be one button **only where an authenticated live/local agent exists**; an offline device cannot be remotely repaired without another independent authorized channel.

Files: breadcrumb navigation, search, preview/read range, user-approved upload/download and progress/resume; no remote private-path auto-discovery. Terminal: reconnect by stable ID, visible retained output, clear detach vs close. Transfers: one source and destination selector, `copy` and resume state; avoid 64 manual MCP turns for a 1-GB file.

### Model/agent tool UX

- Preserve the compact families (`devices`, `exec`, `files`, `terminal`, `jobs`, `transfer`, `copy`, etc.). Use **unique device aliases**; do not make models memorize UUIDs.
- Schemas currently use generic `action: String` plus many optional fields. Introduce action-specific enum validation/schema within an existing family (or a small number of genuinely helpful distinct tools), not dozens of new top-level tools.
- Every tool response: clear `status`, `device`, `operation`, `request_id`, bounded useful data, `next_action` on failures, and `resume_token`/ID for long work. Use `outcome_unknown` for ambiguity, not a generic "error".
- Add accurate MCP tool risk annotations only when the **entire** operation is read-only/idempotent. Never label a mixed read/write family read-only. See https://blog.modelcontextprotocol.io/posts/2026-03-16-tool-annotations/.
- Prefer one task-level action over model-orchestrated 50-step loops: bounded command, status/log summary, cross-device copy. Avoid adding an unrestricted "do everything" tool.
- Keep stable order and concise descriptions for better tools/list caching and selection. Evaluate 2026-07-28 stateless MCP and Tasks extension only after a client-compatibility test. The 2026-07-28 revision allows server-initiated task handles; don't assume legacy opt-in semantics or that the current ChatGPT client supports the extension: https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/changelog.mdx.

## 4. Speed — measure before changing transport

Set up a **repeatable client-visible benchmark**, not arbitrary anecdotal latency. Compare identical operations on the same device in same network: warmed vs new WSS, direct VPN vs public TLS, one-shot exec, files stat/read, terminal echo, device discovery, copy of 1/16/100 MiB, reconnect after network flap. Record p50/p95/p99 (enough samples), timeouts, tool-call count, bytes and CPU/RSS.

Preliminary Mac probes in [CURRENT_REALITY.md](CURRENT_REALITY.md) include fresh connection setup and cannot be used to claim that established WSS is 7× slower than Tailscale.

Low-risk speed choices first:
1. Keep one **persistent multiplexed WSS** and priority heartbeat/control responses (already started).
2. Reduce gateway->agent sequential round trips, JSON serialization and redundant UI polls; coalesce SSE refresh (branch already started).
3. Return limited output by default with cursor/offset and a bounded `read_more`; do not flood model context.
4. Avoid hashing entire huge files unnecessarily on repeated source retries; cache a verified immutable transfer snapshot only if throughput evidence warrants it, with change detection.
5. The new `copy` relays base64 chunks (256 KiB) through the gateway and defaults to 16 MiB per MCP call. That's correct and safe for moderate sizes, **not** a seamless large-file UX. First add an internally durable queued transfer/status handle reusing SQLite + existing agents if tests reveal many manual calls; do not build a new byte-stream transport first.
6. If public-edge data rates are the bottleneck, trial same-origin reverse proxy or a larger bounded chunk/frame size. Benchmark memory, frame size, Cloudflare limits, and power usage before changing any default.

**Proposed acceptance targets (adjust from baseline):** common small operations should use one tool call; repeated warmed read/exec should have p95 below 2× same-host pre-migration baseline excluding model inference; gateway healthy idle CPU should remain low; on an awake desktop, reconnection should happen within roughly 45 seconds (backoff ≤30s + handshake) under restored network. Record deviations and never apply awake-device thresholds to Android Doze.

## 5. Security and release gates

- Merge and test the original **uncommitted OAuth loopback redirects/CSP** into the feature branch without overwriting its other differences, validate authorization code/PKCE, resource binding, refresh rotation, redirect allowlist and issuer behind public TLS. Do not mistake available OAuth routes for deployed public readiness.
- The branch signed updater needs an owner-held Ed25519 key and matching repository variable; signing production assets is not authorized by documentation alone. **Checksum-only bootstrap installers remain an open supply-chain risk**. Complete signed bootstrap verification and anti-rollback policy before auto-updating wide deployment.
- Keep gateway bound loopback, require external TLS, use strict host/origin allowlist, rate-limit public auth/enrollment, and test revoked devices. For public MCP, validate the spec-version compatibility and actual ChatGPT client; avoid blindly switching to stateless protocol.
- Do not expose full command body, SSH private keys, screenshots, clipboard, file contents or export archives in logs, issues or PRs. This repository is presently public.
- No elevated helper until a specific owner workflow cannot be done as a normal user. Never silently bridge Android to ADB/Shizuku/system privileges.

## 6. Concrete order of work — small reviewed PRs

| Sequence | Deliverable | Exit criterion | Deliberately excluded |
| --- | --- | --- | --- |
| **P0-A** | Inventory real Cloudflare route, add loopback-only staging deployment and direct WSS test | Mac signs in with VPN off; edge and gateway health/auth checks; no live disruption | Production route rewrite |
| **P0-B** | Integrate original OAuth edits with the feature worktree; test public MCP auth | Real connected client completes OAuth/PKCE and tool call without loosening CORS/host allowlist | New auth provider |
| **P0-C** | Connection telemetry + Mac and Android lifecycle soak tests | Classified losses, observed p95 recovery, confirmed failure root or narrowed hypothesis | Rewriting WebSocket |
| **P1-A** | One-flow installer + reversible gateway URL migration + UI recovery | Clean device pair -> authenticated connected, no VPN, screen captures and scripted acceptance | Tray/Electron app |
| **P1-B** | Accurate MCP action schemas, capabilities/errors and aliases | One simple task per call; invalid actions fail before dispatch; ambiguous names explicit | 50 new tools |
| **P1-C** | Authenticated first-install bootstrap + signed release/rollback acceptance | Verify producer/public key, no downgrade, macOS/Linux/Windows real update/rollback | TUF implementation unless needed |
| **P1-D** | Durable copy progress/resume for larger files if benchmarks justify | 1-GB copy from UI or MCP with 1 start + bounded status calls; verify digest after forced disconnect | QUIC/data broker |
| **P2** | UX polish, Windows real acceptance, Android Doze tests and legacy cutover | Acceptance matrix green and rollback viable; no duplicated active service | New all-in-one framework |

**At every phase:** keep legacy services and rollback; tests before deployment; no changes to live credentials or base-worktree dirty OAuth sources without first reconciling diffs and gaining confidence in isolated acceptance. Do not use a separate dashboard backend or second MCP deployment merely to add a button.

## 7. Known anomalies to fix deliberately, not through wholesale refactor

- Current gateway has a compiled-in host allowlist containing an old private-network hostname/IP, so moving to loopback/public routing needs deliberate host/origin configuration and security tests.
- The deployed Mac agent still points at a tailnet `ws://` endpoint. It benefits from tailnet encryption, but the future default should be verified public `wss://`. Migrating URL needs stable identity and rollback.
- Current canonical gateway binds tailnet-only. Public HTTPS ingress reaches it somehow, but the locally managed config did not expose that routing rule: verify the remote Cloudflare route in its dashboard. **Do not assume a route target.**
- Some top-level source/docs still describe v1 as "not implemented", OAuth as "pending", Android signing as "not built", or `trash` as "unsupported"; these are historical statements. Keep historical audit records but make this document canonical.
- `apps/mcp-server/` and `apps/host-agent/` are legacy TypeScript paths; `apps/android-agent/` has both signed and legacy HTTP code; several old server services still run. Mark obsolete paths before removing anything.
- The dashboard `app.js` is compact but difficult to maintain. Split by **responsibility** (auth, devices, sessions, files), preserving plain browser JS, rather than adopting a full app framework.
- Device names can collide; the branch addresses routing ambiguity but lacks alias and rename UX.
- Not all advertised capabilities mean current permission: desktop may need TCC or OS prompt, Android permissions are lease-scoped. Do not turn "capability exists" into "usable now".
- Persistent worker spools and signed transfer metadata should have bounded retention/cleanup. Recover from stale `ready`/abandoned uploads with explicit policy.
- The repository is **public**; never add private deployment secrets, routes containing credentials, identifying device identifiers, or raw operational transcripts.
- Avoid release claims from green Linux tests alone. Windows/Android native distribution, boot/wake cycles, production Cloudflare ingress and cross-device copy are separate acceptance gates.
