# Infra: current reality and architecture decisions
Status: **23 October 2026**. Canonical entry point for new coding agents; older dated checklists are historical. This is an evidence-based snapshot, not an instruction to deploy.

## Purpose and non-goals

One lightweight owner-operated control plane for ChatGPT/MCP to use multiple personally authorized machines with low tool-call overhead. A device connects outbound and stays available when its OS/network permits. Never promise unattended control on Android when the OS prevents it. A private network/VPN may be an optional optimization, **not an installation prerequisite**.

Do not rebuild the working Rust backend, implement a second Android MCP server, add privileged permanent daemons, Redis/Postgres, a mobile/desktop GUI framework, or QUIC/HTTP3 merely to improve a checkbox. Different code is justified only if benchmarks show an existing layer cannot meet its real requirements.

## Working components and source of truth

| Area | Current implementation and evidence | Important limit |
| --- | --- | --- |
| Gateway | Rust/axum/tokio, SQLite registry, signed Ed25519 agent challenge, revocation, activity audit, MCP, HTTPS dashboard via edge | Current canonical deployed listener is bound to an Oracle VM **tailnet-only** address; not independent of Tailscale |
| Mac and Linux | Native user agents; startup via launchd/systemd user; PTY, durable jobs, native file/search, transfer, system/network/services/Git/Docker, desktop for Mac | Mac has had intermittent heartbeat timeouts and abrupt WebSocket resets despite reconnecting; actual cause not established |
| Android | Separate Java/Kotlin-side Android app under `apps/android-agent/`, signed outbound gateway connection with narrower capability set | Background/Doze/boot limits, user consent for accessibility and a historical local HTTP helper; avoid assuming shell/root/ADB equivalence |
| MCP | Compact Rust tool families, stateful compatibility and structured JSON responses | `rmcp`/current gateway has not been certified against stateless MCP 2026-07-28; client compatibility matters |
| Dashboard | Embedded HTML/CSS/JS, password/HttpOnly session, terminal attach, files preview, SSE + fallback | Installer flow and actionable recovery need polish; no new SPA needed |
| Auth | Owner password/browser sessions; deployed OAuth routes + signed-agent auth have different trust boundaries | The original primary worktree has **uncommitted** OAuth loopback redirect/CSP edits: do not overwrite or misrepresent as deployed |
| Transfers | Agent-side resumable chunks, SHA-256; branch adds MCP `copy` that resumes with two transfer IDs | Not yet a durable job for huge transfers, not optimized for WAN throughput |
| Releases | Branch adds Ed25519 signed-manifest enforcement to CLI update and CI producer | Signing secrets/public trust root **not provisioned**; bootstrap installers remain checksum-only; anti-rollback incomplete |

### Repository / deployment separation

- The feature branch `fix/infra-ux-reliability-20261023` contains `0a64345` (device resolution, dashboard coalescing, heartbeat overload) and `ff1787e` (cross-device copy, signed native updates, classified disconnect reasons); later context commits may follow. It is a **development candidate**, not installed in production.
- The original worktree `feat/android-agent-plan` is ahead of its upstream branch and has local gateway/OAuth changes plus backups. **Do not reset, merge or overwrite** those files. Reconcile with tests in a fresh worktree before rollout.
- Legacy control services and Desktop Commander continue alongside the Rust service. They are rollback paths, **not** evidence the Rust v1 deployment is absent. Do not stop them without accepted cutover.
- GitHub repository visibility checked during this review is **public**. No live credentials, device UUIDs, private IPs, private log excerpts, personal account metadata, signing keys, enrollment codes or sensitive origin configuration belong in committed context.
- Current public HTTPS health works; unauthenticated MCP and device API return 401. That proves protected endpoints are reachable, **not** that OAuth login or all routes work through the public edge.
- Public HTTPS and the tailnet listener co-exist. The public hostname's actual Cloudflare origin route could not be read from local YAML; verify the remotely managed tunnel route before touching it. The Rust process itself was observed listening only on its tailnet address, not on loopback.

### Observed performance (small, non-production-grade sample)

From the Mac, five fresh `/healthz` HTTP requests to the tailnet Rust listener were approximately 80–105 ms total each. Five fresh public HTTPS probes were approximately 527–735 ms, including TCP/TLS setup, Cloudflare edge and origin routing. This is **not** an established WebSocket latency comparison and not a full MCP benchmark. No evidence supports replacing the transport or asserting a speed improvement yet.

### Recent reliability findings

- Gateway logs showed Mac agent heartbeat timeouts and abrupt socket resets with reconnect shortly thereafter. New feature-branch patches classify disconnects and prioritize the agent's heartbeat over normal outgoing data; they address plausible saturation, not a proven root cause.
- At review time a Mac LaunchAgent and its credential-store verification passed local `infra doctor`, confirming that startup/identity worked at that moment; intermittent transport remains to be measured with correlations against sleep, network switching, gateway restarts and tunnel events.
- The Android app is an actually enrolled signed device, but it also has a legacy local/Tailscale HTTP helper and UX references to a separate "Start agent" button. Do not mistake the helper for the signed protocol channel.
- No source evidence justifies rebuilding v1 or introducing an overlay networking dependency.

## Decision record (as of this review)

1. **Primary connection:** direct outbound `wss://<public-control-host>/agent` on TCP 443 with verified TLS, a persistent socket and the existing application-level signed challenge. No VPN on the Mac/Windows/Linux/Android clients.
2. **Server ingress:** preferred first experiment is the existing Cloudflare Tunnel -> a **loopback-only** Rust gateway on Oracle; reuse existing operational infrastructure. No new public raw agent port. Keep Tailscale as optional maintenance/break-glass and temporary rollback.
3. **Authentication boundaries:** the device signed handshake on `/agent`; short-lived rate-limited enrollment on `/enroll`; owner session on the dashboard; OAuth-protected `/mcp` with explicit issuer/resource/redirect validation. Do not enable public bearer-free control to simplify setup. Cloudflare edge identity challenges can break native WS and MCP clients; configure/test routing deliberately.
4. **Persistence:** one Rust gateway process + SQLite, native desktop agents, Android service. Existing durable worker mechanisms remain. **Do not add Redis, queues, separate streaming server, desktop tray, or third-party message bus.**
5. **Tool interface:** stable unique display names, capability-aware action schema, honest errors and structured minimal responses; reduce model round trips. Never auto-retry an operation with `outcome_unknown`.
6. **UI:** improve embedded dashboard with a one-path Add Device/connection check/repair workflow. Fix legibility of its compact JS incrementally; do not rewrite to React/Electron.
7. **MCP version:** preserve compatibility for the current connected ChatGPT client while evaluating the 2026-07-28 stateless/metadata-header revision in an isolated compatibility harness.
8. **Release integrity:** never claim the new signature scheme covers the first-time installers. Owner-provisioned signing key plus signed bootstrap verification and downgrade protections are distinct gates.

## Evidence pointers

Source: `crates/gateway/src/main.rs`, `oauth.rs`, `store.rs`, `copy.rs`; `crates/agent/src/main.rs`, `transfer.rs`; `crates/cli/src/main.rs`, `release_verify.rs`; `apps/dashboard/`; `apps/android-agent/`; `scripts/smoke-v1.sh` and `scripts/smoke-install-dashboard.sh`.

Historical and platform-specific records: `docs/IMPLEMENTATION_STATUS.md`, `docs/ANDROID_CURRENT_STATUS.md`, `docs/RELEASE_SIGNING.md`, `docs/UX_PRINCIPLES.md`. These include dated claims that may not describe HEAD. Prefer code + tests + live diagnostics, in that order.

External references checked October 2026:
- Cloudflare WSS support and forced edge restarts: https://developers.cloudflare.com/network/websockets/
- Cloudflare Tunnel publish/origin routing: https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/get-started/create-remote-tunnel/
- MCP 2026-07-28 HTTP changes: https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/transports/streamable-http.mdx
- MCP durable Tasks opt-in: https://tasks.extensions.modelcontextprotocol.io/specification/2026-07-28/tasks
- Android foreground-service constraints: https://developer.android.com/develop/background-work/services/fgs/changes
- FCM priority limitations: https://firebase.google.com/docs/cloud-messaging/customize-messages/setting-message-priority

See [NEXT_STEPS.md](NEXT_STEPS.md) for prioritized decisions, staging checks, failure handling, UX goals, and explicit non-goals.
