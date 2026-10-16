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

The current private-deployment mode already separates owner/client authentication from device authentication: MCP requires an admin bearer credential, while the browser exchanges that credential for a 12-hour HMAC-signed `HttpOnly; SameSite=Strict` cookie. The cookie is derived from the admin credential, survives gateway process restart, and becomes invalid when the admin credential rotates. Cookie-authenticated state-changing API calls require a same-origin custom CSRF header. This private mode is not a claim that standards-compatible public OAuth is complete.

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

Implemented now: per-device Ed25519 keys, short-lived one-time enrollment codes stored only as hashes, fresh challenge-response authentication, independent device revocation, connection-generation replacement, protocol heartbeat/stale-socket retirement, and authenticated private-mode gateway access. A non-loopback bind requires a >=32-byte admin credential; MCP uses bearer authentication and protected dashboard APIs accept bearer or a signed browser session. There is no unauthenticated non-loopback override.

The agent identity format is versioned. Version 2 stores only device identity metadata plus a random credential key ID on disk when a native OS credential store is usable; the Ed25519 private signing key is stored through macOS Keychain Services, Windows Credential Manager, or Linux Secret Service. Legacy version-1 identities migrate automatically after a read-back verification of the native store. Real macOS ARM64 acceptance has verified migration to `native_keyring`, removal of the private key from `identity.json`, and signed reconnect. Headless Linux systems without a working Secret Service explicitly use a `0600` private-file fallback. `infra doctor` never retrieves the signing key merely to test it; after successful signed authentication the agent records only the storage mode and verification timestamp in `runtime.json`, which doctor uses as non-secret proof. `infra uninstall --remove-state` removes native credentials before deleting their metadata, and preserves state if native cleanup fails. Standards-compatible public MCP OAuth is still pending.

Persistent PTY workers accept control only on loopback and require a random capability token stored in private session metadata. Structured file access intentionally has the same user-level filesystem reach as the agent; permanent deletion is explicit and recoverable `trash` is not yet implemented.

## 14. Current installer and dashboard boundary

The current bootstrap installers verify SHA-256 release sidecars before installing native binaries. This protects against accidental corruption but is not the final release trust model; signed/authenticated release metadata and steady-state updater rollback remain required. Development `--source-dir` installation intentionally bypasses download verification and must never be presented as the normal remote install path.

The embedded dashboard now has a private-mode login/session boundary. Static assets, health, device enrollment consumption, and the authenticated agent WebSocket remain reachable without an owner browser session because they have their own trust semantics; MCP and device/session/activity/admin APIs are protected. The dashboard posts the admin credential once to `/api/auth/session`, receives an HMAC-signed `HttpOnly; SameSite=Strict` cookie, clears the credential field, and never stores the admin credential in Web Storage. State-changing cookie-authenticated API calls require `X-Infra-CSRF: 1`. A non-loopback gateway will not start without an admin credential, but Infra still expects TLS/Tailscale or a trusted TLS-terminating reverse proxy; the Rust gateway does not yet provide the final public OAuth/TLS edge.

SQLite audit events store only bounded metadata (time, device association, action family, outcome and request ID). Tool payloads, stdout/stderr, file contents, enrollment codes and private device keys are deliberately excluded.

## 15. Structured external-tool boundary

The current structured service/container/Git/listener implementations invoke platform-native commands or established CLIs with direct argv arrays; user-controlled identifiers are never concatenated into a shell command. OS permissions remain authoritative. Linux service mutation is currently restricted to `systemctl --user`; Windows SCM actions may fail when the current user lacks rights; there is no hidden elevation path.

Structured inspection also reduces accidental secret exposure. Docker inspect recursively redacts common password/token/secret/key fields and sensitive environment variables. Git remote HTTP(S) userinfo is removed before returning URLs and common command output URL tokens are sanitized. macOS launchd status returns only selected non-secret status keys instead of raw `launchctl print` output. Explicit logs/diffs can still contain application/user-authored sensitive content and should be requested deliberately.
