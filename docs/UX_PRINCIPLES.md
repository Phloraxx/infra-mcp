# UX principles

Infra succeeds only if it feels invisible during normal use and obvious when something is wrong. Installation, persistence, recovery, status clarity, and session continuity are product features, not post-backend polish.

## Product promise

For a normal owner-controlled device:

1. Add the device once.
2. Install with one obvious action.
3. See it become online automatically.
4. Reboot, sleep, change networks, close ChatGPT, or restart the gateway without manually repairing the agent.
5. Return later and continue from the same device and surviving sessions where technically possible.

The user should almost never need to know about WebSockets, launchd, systemd, Scheduled Tasks, service units, tokens, PIDs, ports, or process supervisors.

## Design rules

### One primary path

Each important action should have one recommended path. Advanced alternatives can exist, but the dashboard and installer should not present several equally weighted choices.

### Make state explicit

Never use a vague binary `online/offline` when the system knows more. User-facing device states should include:

- Online
- Reconnecting
- Sleeping / recently suspended
- Degraded
- Needs permission
- Update available
- Offline
- Revoked

The dashboard should explain the next useful action for degraded states.

### Recovery before documentation

If Infra can detect and repair a common problem safely, expose a `Repair` action instead of sending the user to documentation.

### No secret-copying ceremony

Pairing should use short-lived enrollment codes/links. Device credentials are generated and stored automatically. The user should not copy long-lived agent tokens into config files.

### No runtime dependency puzzle

The supported installer downloads a signed native binary. Node/npm/Python/Homebrew should not be required for normal installation.

### Operations remain understandable

For every consequential action, the activity log should make it possible to answer:

- what happened?
- on which device?
- when?
- did it finish?
- what changed?

Do not log secrets or full sensitive content by default.

### Fast model interaction

UX includes ChatGPT's experience. Common tasks should need few MCP calls and return structured, bounded, actionable responses rather than raw command noise.

## Success criterion

If a technically competent user has to SSH into a device just to repair Infra under a routine failure mode, treat that as a product bug unless the machine itself is fundamentally unreachable.