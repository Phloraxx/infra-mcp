# Infra Fieldwork — visual redesign

**Status (2026-10-24):** Deployed to infra.drbijoy.in following isolated staging and signed-agent reconnection.

The previous generic SaaS card stack is replaced with a **personal operations workspace**. The visual language is quiet, technical and tactile: forest/charcoal navigation, warm mineral surfaces, citron connection signals, editorial typography and fine-lined network geometry.

## Desktop overview

![Infra redesigned desktop overview](previews/overview-desktop.png)

## Mobile overview

![Infra responsive mobile overview](previews/overview-mobile.png)

## Authentication

![Infra private-control-plane sign-in](previews/sign-in.png)

## Interaction and information design

- **Real device topology:** Up to four signed devices with online/offline state and the exact fleet-wide connected count. A topology item opens the corresponding device by stable ID; no fabricated telemetry.
- **Overview hierarchy:** Operations hero → four live measures → offline attention → device inventory → persistent sessions.
- **Inventory discovery:** Name/platform/architecture/capability search, slash shortcut outside editable or terminal contexts, no-match state.
- **Control surfaces:** Consistent form states and touch targets, compact responsive navigation, durable sessions, terminal and file workspaces, device status and account security.
- **Pairing:** Authenticated owner approval, local comparison code and legacy enrollment retained.
- **Security:** Escaped server-provided labels, no third-party font/script/image CDNs, no new permissions or APIs, and the existing browser CSP retained.
- **Responsive:** Browser regressions at 320, 390, 768 and 1280 pixels, system dark-mode preference, motion reduction, navigation, terminal I/O, sessions, file previews, hostile device names, and sign-out clearing.
- **No backend stack change:** The Rust gateway still embeds apps/dashboard/index.html, app.js and styles.css through its existing same-origin frontend routes.

Preview screenshots use fictional devices and sessions, not production data.

## Deployment boundary

**Deployed 2026-10-24:** The existing infra.drbijoy.in gateway service now runs build commit 16ef052 (binary SHA-256 `6913b7c0a683c39262b6d7cebe3baf3e22c87f5551610d746faebf31f3676372`). The earlier uncommitted OAuth loopback callback patch was preserved in this commit without modifying the original worktree. A live SQLite backup and isolated v2-to-v5 migration test preserved all three device identities. The old binary was proven compatible with schema v5 before cutover. An independent guardian performed the restart and validated OAuth, owner authentication, public HTTPS dashboard assets, database integrity and reconnection of both previously online agents. No remote work sessions were active. The Android agent was offline before deployment.

Rollback binary, database snapshot, and audit logs are preserved at `~/.local/share/infra-v1-remote-burnin/gateway/staging-20261024-fieldwork/`. Native device binaries were **not** upgraded. Publisher-authenticated first-install bootstrap, physical Windows reboot/rollback, and long-term soak remain independent release gates.
