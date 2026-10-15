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

## 14. Current implementation notes

Implemented now: per-device Ed25519 keys, short-lived one-time enrollment codes stored only as hashes, fresh challenge-response authentication, independent device revocation, connection-generation replacement, and refusal to bind the unauthenticated development gateway to a non-loopback interface unless an explicit override is supplied.

The agent currently stores its identity in its private state directory; Unix identity files use restrictive permissions. Platform-native Keychain/DPAPI/Secret Service storage is still required before calling credential storage complete. Public MCP/dashboard OAuth is also still pending.

Persistent PTY workers accept control only on loopback and require a random capability token stored in private session metadata. Structured file access intentionally has the same user-level filesystem reach as the agent; permanent deletion is explicit and recoverable `trash` is not yet implemented.

## 14. Current installer and dashboard boundary

The current bootstrap installers verify SHA-256 release sidecars before installing native binaries. This protects against accidental corruption but is not the final release trust model; signed/authenticated release metadata and steady-state updater rollback remain required. Development `--source-dir` installation intentionally bypasses download verification and must never be presented as the normal remote install path.

The embedded dashboard is loopback/private-development infrastructure until browser OAuth/session authentication is implemented. The gateway refuses non-loopback binding by default unless an explicit development override is set. Enrollment creation and revocation support an admin bearer token, while read-only dashboard APIs are not a substitute for production authentication.

SQLite audit events store only bounded metadata (time, device association, action family, outcome and request ID). Tool payloads, stdout/stderr, file contents, enrollment codes and private device keys are deliberately excluded.

## 15. Structured external-tool boundary

The current structured service/container/Git/listener implementations invoke platform-native commands or established CLIs with direct argv arrays; user-controlled identifiers are never concatenated into a shell command. OS permissions remain authoritative. Linux service mutation is currently restricted to `systemctl --user`; Windows SCM actions may fail when the current user lacks rights; there is no hidden elevation path.

Structured inspection also reduces accidental secret exposure. Docker inspect recursively redacts common password/token/secret/key fields and sensitive environment variables. Git remote HTTP(S) userinfo is removed before returning URLs and common command output URL tokens are sanitized. macOS launchd status returns only selected non-secret status keys instead of raw `launchctl print` output. Explicit logs/diffs can still contain application/user-authored sensitive content and should be requested deliberately.
