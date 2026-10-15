# Security model

## 1. Security posture

Infra v1 intentionally provides broad user-level control of authorized computers. The security objective is therefore:

> Only an authenticated owner through an authenticated gateway can instruct an authenticated device agent, and the agent cannot silently exceed the permissions of its normal OS identity unless a separately installed privileged helper authorizes a defined elevated operation.

This is different from the old restricted-operation prototype model.

## 2. Trust boundaries

```text
ChatGPT / MCP client
        |
        | OAuth/TLS
        v
infra-gateway
        |
        | authenticated encrypted device channel
        v
normal user agent
        |
        | authenticated local IPC (optional)
        v
privileged helper
```

The gateway and agent are trusted components, but compromise of one should not automatically imply permanent credential access to every other device.

## 3. Threats to consider

- stolen/replayed device credential
- stolen gateway OAuth/session token
- malicious or accidental model command
- command injection inside structured operations
- path traversal in file tools/transfers
- compromised update server/artifact
- stale socket preventing legitimate reconnect
- local unprivileged process impersonating the agent to the privileged helper
- audit log leaking secrets
- screenshot/clipboard data exposure
- unsafe automatic retry of a consequential command

## 4. Device identity

Preferred design:

- per-device asymmetric keypair generated locally
- public key registered at gateway
- device identity independently revocable
- private key stored using platform-protected storage where possible
- short-lived connection authentication derived from/signing challenges rather than transmitting a reusable master secret each reconnect

Pairing codes must be short-lived, one-time, rate-limited, and bound to a pairing request.

## 5. Gateway authentication

MCP client authentication is separate from device authentication.

Use standards-compatible OAuth/token validation for the public MCP endpoint. Do not treat the internal device credential as user authentication.

For personal deployment, keep scope design simple but explicit, for example:

- read/control owner session
- device administration/pairing

Do not build a complex enterprise RBAC system unless needed.

## 6. Privilege separation

The normal agent runs in the user's context.

The privileged helper:

- has no public network listener
- communicates only over local authenticated IPC (Unix socket/named pipe/XPC-style mechanism as appropriate)
- validates the normal agent/caller identity
- supports explicit elevated operations
- logs metadata for elevated actions
- can be omitted entirely on machines where user-level access is enough

Do not expose a hidden unrestricted root shell merely for implementation convenience.

## 7. Shell execution

User-level shell execution is first-class and should be represented honestly.

Security requirements:

- command sent only to explicitly selected authenticated device
- execute in user context unless an elevated API is explicitly invoked
- preserve stdout/stderr separation where possible
- do not automatically retry after an ambiguous disconnect
- audit command metadata carefully; consider configurable redaction to avoid persisting secrets supplied on command lines

## 8. Filesystem safety

The user-level agent may access anything its OS user can access. Structured file operations still need:

- canonical path handling where relevant
- defense against transfer temp-file traversal
- atomic replacement
- symlink-aware design decisions
- explicit permanent-delete action
- limits for reads/search/transfer concurrency

## 9. Secret handling

Never log:

- private device keys
- refresh tokens
- pairing secrets
- OAuth client secrets
- passwords
- raw credential-store values

Tool output itself can be sensitive and may enter the ChatGPT conversation. The agent should avoid “helpful” automatic secret discovery unless the user explicitly asks for it and policy allows it.

## 10. Updates

Auto-update is part of the trust chain.

Required properties:

- signed/authenticated manifest or release metadata
- cryptographic hash verification
- architecture/platform match
- staged replacement
- post-update health check
- rollback to previous known-good binary
- no executing arbitrary unsigned downloaded scripts as the steady-state updater

Bootstrap installers may be scripts, but installed steady-state updates should be stronger.

## 11. Audit

Audit records should contain:

- time
- device ID
- operation/tool
- request/trace ID
- high-level target (path/process/container/etc.) where safe
- outcome and duration
- privileged flag
- bytes transferred where useful

Do not store full command output/file contents by default.

## 12. Safety classification

Each structured operation must be classified:

- read-only
- reversible write
- consequential
- destructive
- privileged

Classification informs MCP annotations and UI/confirmation behavior but does not replace authentication.

## 13. Recovery

If a device is lost:

- revoke it at gateway
- invalidate device key/public-key registration
- remove active sessions/transfers

If gateway credentials are suspected compromised, rotate them without requiring all device filesystems to be reinstalled when possible.
