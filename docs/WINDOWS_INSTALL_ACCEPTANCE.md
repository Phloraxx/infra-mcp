> **Updated UX contract:** see [ONBOARDING_REDESIGN.md](ONBOARDING_REDESIGN.md) for replacing the PowerShell happy path with a signed per-user package and device-initiated owner approval.

# Windows Infra installation — current acceptance

**2026-10-24 update:** The Windows CI startup task and newly added device-initiated pairing scenario now pass on an isolated Windows runner, including an installation path with spaces, genuine OS hostname, key-bound owner approval, setup retry/identity preservation and signed online reconnection. Fix: use structured ScheduledTasks action/executable/arguments instead of the fragile `schtasks /TR` string. This is not physical-PC logon/reboot acceptance and does not mean a signed public release exists. See [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).

**2026-10-23:** Windows x64 native Rust build and ConPTY test suite passed in GitHub Actions, but physical Windows/Rose enrollment, signed gateway handshake, reboot reconnect and updater are **not yet certified**. The device is not currently listed as connected to the canonical gateway. Another ChatGPT Windows troubleshooting session was referenced by the owner but its exact errors were not available to this worktree review; do not state a root cause based on those unseen logs.

## Verified blocker: no release to install

The GitHub repository currently has **no published GitHub Releases**. The production-oriented `install.ps1` defaults to `https://github.com/Phloraxx/infra-mcp/releases/latest/download/infra-windows-x86_64.exe` (or ARM64 equivalent). That URL has no release asset at present. Compiled CI acceptance artifacts and source-tree binaries are **not** a tagged release.

Fix in candidate branch: make Windows installer preflight both binary assets before touching the install folder, and explain a missing release rather than presenting a generic PowerShell 404. Do **not** publish or install untrusted binaries merely to make the link work; initial-release publisher authenticity is separately gated in [RELEASE_SIGNING.md](RELEASE_SIGNING.md).

## Candidate Windows staging acceptance

`scripts/smoke-install-windows.ps1` is run by `.github/workflows/rust.yml` on an isolated `windows-latest` runner under Windows PowerShell 5.1. It verifies:
- missing second binary fails without modifying installation/state;
- paths with spaces, two-binary local source staging, user-level `--no-startup`;
- ephemeral loopback gateway, one-time enrollment, persisted identity and configuration;
- real Windows native `infra.exe status --json`, then a signed agent connection;
- test processes and temporary files removed; no real Scheduled Task or production account touched.

The dashboard now generates Windows bootstrap commands using `Invoke-WebRequest -UseBasicParsing`. Microsoft made that explicit option important for unattended Windows PowerShell 5.1 web requests after its December 2025 security update; see https://support.microsoft.com/KB/5074596.

**This is not a production validation.** Physical Windows acceptance still requires the actual failure details, Windows agent/service diagnostics, account/security context, startup test, sleep/reboot and actual remote MCP calls. Do not reinstall/repair/restart another user's Windows machine or remove existing fallback tools without explicit consent.

## Physical Windows troubleshooting (owner-side, no secret copying)

In a PowerShell window under the intended user account (not necessarily elevated), on a machine with the CLI already installed:

```powershell
& "$env:LOCALAPPDATA\Infra\bin\infra.exe" status --json
& "$env:LOCALAPPDATA\Infra\bin\infra.exe" doctor --json
Get-ScheduledTask -TaskName 'Infra MCP Agent' -ErrorAction SilentlyContinue |
    Select-Object TaskName, State, TaskPath
```

If the CLI is not installed, check whether an official signed/tagged release has been published. Do not reuse stale enrollment codes; generate a fresh one in the authorized dashboard. Separate and sanitize sensitive values before sharing diagnostics.

## Required outcomes

1. Windows native source/CI staging test passes.
2. Tagged, authenticated first-install release assets become available.
3. A real Windows machine uses a reachable `wss://` gateway without mandatory Tailscale.
4. Windows scheduled startup launches and reconnects after login/reboot, with explicit launch errors.
5. Known-good agent can execute a minimal user-level command, start and reattach a ConPTY and transfer a small file, with no stale registry duplicates.
6. Updater stages, checks health, rolls back a deliberately bad version and leaves previous identity intact.
