# Infra v1 deployment direction

Infra v1 is not implemented yet. This document defines how development and eventual rollout should work without breaking the current remote-control setup.

## 1. Do not replace v0.3 in place during development

The working Node-based deployment remains the fallback until v1 passes the replacement benchmark.

Run v1 with separate:

- service names
- state/config directories
- gateway listener/port or staging hostname
- device credentials
- logs

Do not point the existing production installer/update path at unfinished v1 binaries.

## 2. Initial development deployment

The first target is the existing Linux ARM64 Oracle host because it can run gateway and a local test agent side-by-side.

Recommended temporary components:

```text
infra-v1-gateway
infra-v1-agent
```

The gateway should initially bind privately/locally and be exposed to ChatGPT only through an authenticated staging path.

## 3. Gateway production shape

Eventually the gateway needs:

- TLS termination/reverse proxy
- authenticated MCP Streamable HTTP endpoint
- persistent SQLite volume
- agent listener
- release/update metadata endpoint if self-hosted updates are used
- health/readiness endpoint

The gateway itself should not require Docker, but container deployment on Oracle is acceptable if operationally convenient.

## 4. Agent installation shape

Final installation should converge toward:

### macOS/Linux

```bash
curl -fsSL https://<gateway>/install | sh -s -- <pair-code>
```

### Windows

A signed PowerShell/bootstrap command or signed installer package that downloads the correct native binary and pairs it.

The steady-state installed agent should not require Node/npm.

## 5. Local CLI

Desired commands:

```text
infra status
infra start
infra stop
infra reconnect
infra doctor
infra logs
infra update
infra repair
infra uninstall
```

Exact command surface may evolve, but troubleshooting must remain simple.

## 6. Startup

- Linux: systemd user service by default; system service only when specifically selected.
- macOS: launchd user agent.
- Windows: robust user startup for normal mode; Windows service for privileged helper/system mode.

Installers must verify that the agent actually reconnects before claiming success.

## 7. Updates

Production updates should:

1. fetch authenticated release metadata.
2. download the correct platform/architecture artifact.
3. verify signature/hash.
4. stage beside current binary.
5. restart into new version.
6. run health/connection check.
7. roll back automatically if the new version fails.

## 8. Rollout order

1. Linux ARM64 canary.
2. macOS ARM64.
3. Windows x64.
4. enable v1 as the default MCP during a burn-in period.
5. benchmark against Desktop Commander.
6. remove old services only after explicit cutover.

## 9. Secrets

Never commit live:

- device private keys
- bearer/bootstrap tokens
- OAuth credentials
- gateway signing keys
- update signing private keys

Example configuration in Git must use placeholders only.

## 10. Legacy deployment documentation

The previous documentation described a restricted Docker/systemd diagnostics gateway and no longer represents the v1 target. Refer to Git history if that deployment model must be reproduced.
