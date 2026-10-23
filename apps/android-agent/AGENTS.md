# Android agent contributor context

> **2026-10-23 current context:** This branch contains an already-enrolled and phone-tested Android v0.3.0 agent; the opening historical v0.1 prototype wording below is not its current runtime state. Newer **v0.3.1 source on the owner's Mac has not yet been reconciled with this GitHub module**. Before implementing Android privileges, read [source handoff](../../docs/ANDROID_HANDOFF_2026-10-23.md) and [embedded ADB research/plan](../../docs/ANDROID_ADB_RESEARCH_AND_PLAN.md). Never infer device installation from APK build success.

This subdirectory is a **prototype Android APK source import**, not the production Infra agent yet.

## Mandatory reading
0. `../../docs/ANDROID_HANDOFF_2026-10-23.md` — source-of-truth discrepancy and safe v0.3.1 reconciliation.
0. `../../docs/ANDROID_ADB_RESEARCH_AND_PLAN.md` — proposed opt-in local ADB client, owner lease, no automatic elevation.
1. `../../AGENTS.md` — Infra v1 principles and live deployment separation.
2. `../../docs/ANDROID_AGENT_PLAN.md` — rationale, phases, product constraints.
3. `../../docs/ANDROID_PROTOCOL_ADAPTER.md` — exact gateway signed-auth / wire compatibility.
4. `../../docs/ANDROID_SECURITY_AND_TESTS.md` — threats, permissions, acceptance.
5. `../../docs/PROTOCOL.md` and Rust `../../crates/protocol/src/lib.rs` — authoritative wire types.
6. `../../crates/gateway/src/main.rs` — enrollment and `/agent` authentication.
7. `../../crates/agent/src/identity.rs` — Ed25519 signature byte layout.

## Do not confuse layers
- Current `AgentService.java` is an inbound HTTP prototype, not the final signed outbound Infra protocol agent.
- Android must become a first-class device in **existing Rust gateway**; no second MCP server is needed on the phone.
- Android API permissions are **not** ADB/root and cannot read arbitrary app sandboxes.
- Do not deploy or restart the live Node v0.3, canonical Rust gateway, Cloudflare rules or Tailscale settings just to develop Android.
- Never commit real device IDs tied to account ownership, private keys, enrollment codes, bearer tokens, addresses, passwords or personal data.

## Engineering targets
- Java/Kotlin Android implementation with existing Rust protocol compatibility tests.
- Signed Ed25519 handshake over verified WSS; use AndroidKeyStore AES-GCM wrapping for private Ed25519 seed where supported.
- Runtime advertised capabilities derived from OS grants; explicit `unsupported`/`unauthorized` for other operations.
- No generic remote exec shell in MVP. Termux and UI automation are later explicit opt-in work.
- Foreground service Start/Stop, revocation, user-visible access and Android process lifecycle first-class.
- Android SAF permission tree containment and app-private workspace mandatory.
- No auto boot/background privilege tricks, root exploits, or permanent hidden connectivity.
- Do not keep the unauthenticated static `/site/` listener after signed direct-agent migration without adding authorization.

## Baseline
As of Oct 18, 2026, `./gradlew :app:assembleDebug` produced a debug APK on Mac with Android SDK 35 and Java 21; installed on Motorola Edge 60 Stylus Android 16. Authenticated local API responded; file upload and traversal denial checked; Oracle reached the site over Tailscale. **Signed direct gateway integration has not been implemented or tested.**

## Proposed elevated Android track (not implemented)

Direct on-device wireless ADB is a **research spike**, not current `exec` semantics. Existing signed WSS/auth, workspace and transfers remain available without developer options. Keep any future ADB pairing identity distinct from Infra device keys; do not expose a public ADB listener, silently install applications, or grant general shell authority merely because a gateway request is signed. First reconcile the newer Mac Android source, then test phone-only pairing + read-only `uid=2000(shell)` under explicit local approval. No Shizuku clone or new MCP server without evidence. Follow the dedicated plan's security/rollback gates.
