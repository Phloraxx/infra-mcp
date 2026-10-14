# Security model

Infra MCP intentionally provides owner-authorized shell and filesystem control. A user agent has the permissions of its installation account; a system agent can have root or SYSTEM privileges. This is a personal, single-owner product, not a multi-tenant hosting service.

The active implementation is in `runtime/`. Older scaffold documentation describing an allowlisted diagnostics-only agent does not describe this runtime.

- Keep the gateway behind HTTPS. Its listener binds to loopback.
- Protect the dashboard with its configured password and the MCP endpoint with OAuth/PKCE. Keep signing secrets stable and outside Git. Access, refresh and dashboard tokens have separate purposes; refresh/dashboard tokens cannot call MCP tools.
- Pair new devices using an expiring, single-use code. Each device receives a separate credential. Device removal revokes that credential and disconnects its socket.
- The legacy shared agent token is retained for existing devices during migration; replace those installations with per-device credentials.
- Restrict local credential files and avoid logging passwords, tokens or signing secrets. Dashboard/API responses omit device credentials.
- Tools act only on the named connected device. Responses are associated with that device's socket. Local agents use an installation lock to prevent duplicate processes.
- Command output and file contents can contain sensitive data and become part of the ChatGPT tool context. Output and saved job logs are bounded, but the owner remains responsible for what they request and share.
- Connection loss does not guarantee a command stopped. Inspect its job status before rerunning it. Arbitrary work is not automatically replayed after a reboot.
- GUI access still requires the relevant OS permissions and an available user session. Root/SYSTEM access does not bypass protected login screens or Android application isolation.

Report reproducible security problems privately to the repository owner, with sensitive data removed from logs and screenshots.
