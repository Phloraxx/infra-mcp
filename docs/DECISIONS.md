# Architecture decisions

This is a lightweight ADR index. “Locked” means implementation should follow it unless the user deliberately changes the direction.

## D001 — Rebuild v1 core in Rust

**Status:** Locked

Both the gateway and device agent move to Rust. The existing Node implementation remains a migration reference/fallback.

Reason: native distribution, lower runtime dependency burden, stronger control over processes/PTYs/streams, and one cross-platform systems codebase.

## D002 — Arbitrary user-level shell is a core feature

**Status:** Locked

The old restricted allowlist-only security model does not match the product goal. Authorized user-level command execution is intentionally supported.

Elevation remains separate.

## D003 — Separate privileged helper

**Status:** Locked

Do not run the normal agent permanently as root/SYSTEM merely to enable occasional admin operations.

## D004 — Compact MCP surface

**Status:** Locked

Expose coherent tool families instead of a large collection of tiny tools. Structured common operations coexist with shell escape hatch.

## D005 — MCP gateway uses SQLite

**Status:** Locked for personal v1

SQLite is sufficient for device/session/job/audit metadata. Do not introduce Postgres/Redis unless a proven requirement appears.

## D006 — WSS first, transport abstraction from day one

**Status:** Locked

Ship the initial agent channel over WebSocket/TLS for deployability. Protocol semantics must not depend on WebSocket so QUIC/WebTransport can be added later.

This supersedes the earlier idea of making QUIC mandatory before parity.

## D007 — Persistent PTY is foundational

**Status:** Locked

Interactive terminal sessions are first-class resources with IDs, offsets, attach/detach, and signals. They are not emulated by repeated one-shot commands.

## D008 — Per-device asymmetric identity is the target

**Status:** Locked direction

Pairing remains simple, but long-lived device authentication should use a locally generated keypair rather than one reusable bearer token as the final design.

## D009 — Semantic desktop automation first

**Status:** Locked direction

When desktop control is implemented, accessibility/UI automation is preferred over coordinate-only clicking. Visual screenshots are complementary.

## D010 — Headless core, optional tray UI

**Status:** Locked

Servers must run the same core agent without a GUI. A Tauri/native tray application may improve desktop UX later but is not part of the execution core.

## Open implementation choices

These remain intentionally open until prototypes/tests justify a choice:

- exact Rust MCP SDK/library
- exact WebSocket/HTTP stack within the Rust ecosystem
- `rusqlite` vs async SQLite wrapper
- exact PTY crate vs direct OS code
- exact secure-store crate/bindings
- exact update-signing framework
- exact macOS privileged-helper/native shim approach
- whether/when QUIC becomes default transport


## D011 — Android as a first-class signed device using the existing gateway

**Status:** Proposed direction, 2026-10-18; direct-agent transport and gateway enrollment NOT yet implemented in the APK.

Implement outbound Android `wss://.../agent` using Infra's existing signed Ed25519 device identity/enrollment and Rust Serde wire protocol; surface the phone under existing MCP `devices` rather than shipping a separate phone MCP endpoint or retaining the Oracle HTTP helper. Android app-specific sandbox and per-feature permissions constrain capability advertisements; desktop user-shell assumptions do not apply. Foreground-service and screen permission restrictions must be designed and tested, not bypassed. Choose Java/Kotlin for the Android agent lifecycle and protocol compatibility tests first; a Rust/JNI core is an optional later optimization.

**Tradeoffs:** maintain a small cross-language wire adapter + golden tests instead of introducing Rust/JNI packaging complexity now; rely on Android OS/background limits instead of promising persistent availability; use per-device strong signing identity rather than perpetual bearer token; use user's SAF selection instead of blanket storage privileges.

See [ANDROID_AGENT_PLAN.md](ANDROID_AGENT_PLAN.md), [ANDROID_PROTOCOL_ADAPTER.md](ANDROID_PROTOCOL_ADAPTER.md) and [ANDROID_SECURITY_AND_TESTS.md](ANDROID_SECURITY_AND_TESTS.md).

## D012 — Public outbound WSS with no required VPN (2026-10-23)

**Status:** Preferred staging experiment; production cutover not approved.

Keep the Rust gateway, existing signed agent protocol and SQLite. A public HTTPS/WSS hostname on port 443 terminates at an existing gateway ingress, with the Rust gateway bound to loopback, while native/Android devices make outbound signed WSS connections. Default enrollment must not require Tailscale or an inbound port on devices.

First test the existing Cloudflare Tunnel. It already exists, supports WebSockets, and avoids a new public origin listener. Benchmark against an existing direct HTTPS reverse proxy if the edge dominates warmed operations or causes unacceptable reconnect churn. Tailscale remains an optional recovery/maintenance path. Neither Cloudflare Access cookie challenges nor broad anonymous MCP permissions may be added to the signed agent path.

Do not replace with QUIC/WebRTC, run a new persistent relay service, or change current production endpoints before staging/rollback acceptance. See [CURRENT_REALITY.md](CURRENT_REALITY.md) and [NEXT_STEPS.md](NEXT_STEPS.md).

## D013 — UX and observability before another rewrite (2026-10-23)

**Status:** Locked direction.

One pairing flow, one unique human-readable device alias, actionable diagnostic states, structured errors, bounded outputs and resumable handles. Avoid a new SPA/runtime, a tray app, and extra tool families unless measured model/tool performance requires them. Use existing Rust tracing, SQLite, native agents and the embedded dashboard.

Do not conflate an OS capability advertisement with runtime permission, or a disconnected client with a remotely repairable process. Treat `outcome_unknown` as a first-class UX state that blocks blind mutation retries.

## D014 — Signed updates are a staged security feature (2026-10-23)

**Status:** Code exists in feature branch; release trust root/first-install bootstrap incomplete.

Feature code can verify Ed25519-signed update manifests and asset hashes, but neither private release-signing key provisioning nor signed bootstrap authenticity and downgrade protection are complete. Public distribution and unattended update are **not** ready until their own security and OS acceptance gates pass. See [RELEASE_SIGNING.md](RELEASE_SIGNING.md).


## D015 — Rebuild first-run onboarding, preserve native execution core (2026-10-24)

**Status:** Product direction proposed for implementation; not deployed.

The current dashboard name field, one-time code before installation, multi-flag PowerShell command, and "installed" message without actual signed online readiness fail the expected nontechnical owner journey. The verified Windows trial hit a null-valued script download and an unreachable installer host. Further first-run patches cannot repair the basic workflow shape.

Use a **signed per-user Windows MSI** as the first supported primary installation surface (after a real release/publisher trust root exists), plus the existing Rust CLI to open a browser-based, device-initiated, owner-approved pairing flow. Detect OS hostname locally, allow renaming as a separate server-side alias later. Keep a text bootstrap only under Advanced/headless. Stop claiming completion before signed agent handshake and status probe. Pairing must resume without reinstall.

Preserve existing Rust gateway, Ed25519 identities, SQLite, MCP and PTY/job/transfer logic. Do **not** add Electron/Tauri, a second gateway, or a new always-running onboarding service. Pair approval lives in the existing gateway and SQLite; the CLI can temporarily handle pending enrollment and open the browser. Implement the independent local verification code with the lightest safe UI proven by a Windows prototype, not an elevated MSI custom dialog by assumption.

Detailed protocol, threat model, UX storyboard, migration and acceptance tests: [ONBOARDING_REDESIGN.md](ONBOARDING_REDESIGN.md).

## D016 — Make one-command setup the first release priority (2026-10-24)

**Status:** Proposed; supersedes the MSI-first sequence of D015, not the pairing security model.

Use small Windows PowerShell and POSIX adapters to verify a published release, then run one shared Rust setup/repair lifecycle. Detect hostname automatically. Require authenticated owner approval and proven signed agent readiness before displaying Connected. Preserve identity and the previous binary on failures. MSI/PKG packaging is optional later. See [ONE_COMMAND_SETUP.md](ONE_COMMAND_SETUP.md).
