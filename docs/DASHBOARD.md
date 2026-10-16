# Dashboard product design

The dashboard is a control/status surface, not a replacement for ChatGPT. It should make setup, health, recovery, permissions, sessions, and audit history obvious while keeping routine operation minimal.

## Information architecture

Keep the global navigation small:

1. **Overview**
2. **Devices**
3. **Sessions**
4. **Activity**
5. **Settings**

The primary CTA is **Add device**.

## Overview

The home screen should answer three questions immediately:

- Are my devices reachable?
- Is anything requiring attention?
- What is currently running?

Recommended sections:

### Device summary

Compact cards/rows showing:

- friendly device name
- OS/icon and architecture
- connection state
- last seen when not connected
- agent version/update state
- key capabilities (Shell, Files, Desktop, Elevated)
- active session/job count

Do not overload cards with CPU charts and detailed telemetry unless there is an actionable reason.

### Needs attention

Only show this section when relevant. Examples:

- Accessibility permission missing
- agent outdated
- startup integration broken
- repeated reconnect failures
- privileged helper unhealthy
- credential/key-store problem

Each item should have one primary corrective action.

### Active work

Show running PTYs, jobs, and transfers across all devices with attach/cancel controls.

## Device detail page

Tabs:

### Overview

- connection status and quality
- hostname/friendly name
- OS/version/architecture
- uptime and current boot ID
- agent/gateway compatibility
- install/startup mode
- capabilities and permissions
- last successful heartbeat
- optional concise system health

Primary actions: `Reconnect`, `Run doctor`, `Update`, `Remove`.

### Terminal

A human terminal is useful for diagnostics and testing, but not the primary product. It should attach to the same persistent PTY system used by MCP.

### Files

Simple browse/download/upload capability. Avoid building a full IDE.

### Sessions

List persistent PTYs, jobs and transfers with state, start time, owner/request origin, retained output size, and controls.

### Permissions

Show capability-by-capability status:

- shell/files
- screen capture
- accessibility/input automation
- clipboard
- privileged helper

For OS permissions, show the exact local action needed and detect when permission changes.

### Diagnostics

Human-readable `doctor` output, recent reconnect history, version details, startup registration state, and exportable sanitized diagnostics.

### Activity

Device-scoped audit history.

## Sessions page

Global view of:

- interactive terminals
- background jobs
- file transfers

Filters by device/state/type. Sessions should remain discoverable after ChatGPT/browser refresh.

## Activity page

Searchable timeline containing bounded metadata:

- timestamp
- device
- action/tool family
- status
- duration
- request/session ID
- initiator/client

Do not store command stdout, arbitrary file contents, secrets, or clipboard contents in the audit database by default.

## Settings

Keep settings focused:

- gateway/public endpoint status
- connected ChatGPT/MCP clients
- device enrollment policy
- update channel
- retention limits
- security/audit options
- advanced transport settings hidden behind an Advanced section

## Visual behavior

Use a restrained, utility-first interface. Avoid decorative dashboards, fake charts, excessive gradients, and dense card grids.

Status indicators must not depend on color alone; pair color with text/iconography.

Use progressive disclosure: advanced networking/protocol details belong in diagnostics, not the Overview page.

## Real-time updates

The dashboard should update device state, enrollment progress, active sessions, and repair/update progress without manual refresh, using a server-sent or WebSocket event channel.

## Mobile behavior

Mobile dashboard should support monitoring, approving enrollment, reconnect/repair/update actions, session inspection, and basic terminal attachment. File management and advanced diagnostics can remain desktop-optimized.

## Destructive actions

Removing/revoking a device, killing a session, uninstalling remotely, or performing privileged destructive actions must clearly identify the target device and consequence before execution.
## Implemented foundation — 2026-10-15

The gateway now embeds a dependency-free dashboard under `/` with Overview, Devices, Sessions, Activity and Settings, responsive/dark-mode styling, Add Device enrollment and OS-specific bootstrap commands. Private-mode browser authentication exchanges the admin credential once for a 12-hour HMAC-signed `HttpOnly; SameSite=Strict` session cookie; the credential input is cleared and no admin secret is retained in `sessionStorage`/`localStorage`. `/api/events` uses the same authenticated browser cookie and server-sent events to refresh state immediately, with periodic polling as fallback.

The dashboard is backed by persisted `/api/devices`, reconciled `/api/sessions`, and bounded `/api/activity` metadata. The activity store records action family, outcome, request ID, timestamp and device association; it intentionally does not persist command output, arbitrary file contents or enrollment/private-key material.

This is the control/status foundation, not the finished dashboard. Device-detail actions, human terminal attachment, file browser/transfer controls, permission onboarding, doctor/repair UI, update progress, standards-compatible public OAuth and richer attention states remain. Private-mode bearer/cookie authentication is implemented; Internet-facing deployment still requires the final OAuth/TLS edge rather than treating the admin credential as a public identity system.
