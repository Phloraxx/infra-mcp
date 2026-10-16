# Development guide for Infra v1

This document describes the development workflow for the active Rust implementation.

## Branching

Keep `main` buildable. Use focused feature branches/PRs for protocol, agent, gateway, and platform work. Avoid giant “rewrite everything” commits once implementation begins.

The documentation transition may land before code; that is intentional.

## Rust baseline

Use the current stable Rust toolchain when implementation starts. Commit a `rust-toolchain.toml` only if pinning is needed for repeatability.

Expected checks:

```text
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Add stricter linting deliberately rather than enabling large warning sets that create busywork.

## Workspace boundaries

Recommended:

```text
crates/protocol
crates/gateway
crates/agent
crates/platform
crates/privileged-helper
```

Keep protocol types independent from gateway HTTP/MCP types and from OS-specific implementation details.

## Configuration

Use explicit config files/environment variables for deploy-time settings, but do not require environment variables for every local preference.

Never commit secrets. Provide examples with obvious placeholders.

Suggested config concepts:

Gateway:

- bind/listen address
- public MCP URL
- database path
- private-mode admin credential
- OAuth settings for the future public edge
- agent listener URL
- logging level

Agent:

- gateway URL
- device identity/key reference
- friendly name
- spool/state directory
- logging level
- update channel

## Logging

Use structured tracing.

Logs should include:

- component
- request/trace ID
- device ID where relevant
- operation
- duration/outcome

Logs should avoid full terminal/file payloads and secrets by default.

## Testing pyramid

### Unit

- protocol serialization/validation
- path/limit handling
- retry classifications
- state transitions

### Integration

- gateway + simulated agent
- agent + test gateway
- reconnect/supersession
- PTY lifecycle
- transfer resume
- SQLite migration/restart

### Real-device smoke

Run against actual Linux/macOS/Windows devices for platform-specific behavior.

### Failure injection

Test:

- kill gateway mid-command
- kill agent mid-transfer
- network disconnect
- stale connection remains half-open
- disk full while spooling/writing
- permission denied
- malformed protocol frames

## Protocol fixtures

Keep versioned handshake/message fixtures so refactors cannot accidentally change the wire protocol without review.

## Compatibility

During migration, v1 does not need wire compatibility with the Node v0.3 agent unless implementing a temporary bridge substantially reduces rollout risk. Behavioral parity is more important than preserving an ad-hoc wire format.

## Performance work

Measure before optimizing. Preserve useful benchmark scripts/results under a dedicated `bench/` or `tests/benchmarks/` area once they exist.

Avoid dependencies that add large resident memory/runtime requirements for convenience only.

## Documentation rule

If an implementation decision contradicts a locked decision in `DECISIONS.md`, update the decision intentionally in the same change rather than allowing code and docs to diverge silently.

## Isolated end-to-end v1 smoke test

Run from the repository root:

```bash
./scripts/smoke-v1.sh
```

The script builds only v1 binaries, allocates a temporary loopback gateway/database/state directory, creates a short-lived enrollment, starts a real agent, verifies it becomes online, calls MCP `exec`, restarts only that temporary controller, and verifies identity-based signed reconnect without an enrollment code. Everything is removed on exit. It does not inspect, stop, modify, or reuse the live v0.3 deployment.

Use this after protocol/enrollment/session changes in addition to the normal formatting, clippy and test commands.

## Installer/dashboard smoke test

Run:

```bash
./scripts/smoke-install-dashboard.sh
```

This builds the v1 gateway, agent and `infra` CLI, starts an isolated loopback gateway with a random admin credential, verifies unauthenticated protected APIs fail, signs the browser into an HttpOnly session, verifies embedded dashboard/bootstrap assets, creates an enrollment, runs `scripts/install.sh` in `--source-dir` development mode with startup intentionally disabled, verifies config and identity persistence, starts the installed agent without the enrollment code, then verifies status/doctor plus authenticated sessions/activity APIs. It also checks logout returns protected APIs to 401. Everything is removed on exit and it does not register a real service or touch v0.3.

`./scripts/install.sh --source-dir target/debug ...` and the analogous PowerShell `-SourceDir` are development-only paths. Normal release installs download versioned native assets plus `.sha256` sidecars. `.github/workflows/release.yml` defines the expected release asset names for Linux x64/ARM64, macOS Intel/ARM64 and Windows x64/ARM64. A real release must be published and validated before treating the remote bootstrap path as production-ready.

## Linux real-service burn-in

On an actual Linux user session with systemd, build release binaries and run the side-by-side burn-in:

```bash
cargo build --release -p infra-gateway -p infra-agent -p infra-cli
./scripts/burnin-linux-user.sh install
./scripts/burnin-linux-user.sh test
./scripts/burnin-linux-user.sh status
```

The burn-in uses a loopback-only gateway and state under `~/.local/share/infra-v1-burnin`, never the live v0.3 gateway. It validates the real startup adapter, including remote shells that lack user-bus environment variables, idempotent reinstall, signed reconnect, MCP exec, and **fresh** PTY input/output after both gateway and agent-controller service restarts. It also starts a durable background job before the controller restart and requires the same job ID to produce a later marker afterward. Use `./scripts/burnin-linux-user.sh remove` to remove only the v1 burn-in.

This is intentionally stronger than re-reading a pre-restart transcript: persistence only passes when the same PTY accepts and returns a new marker after the restart.

## PR native acceptance artifacts

The Rust PR matrix builds release-mode `infra` and `infra-agent` binaries after the macOS ARM64 and Windows x64 jobs pass, then uploads them as short-lived GitHub Actions artifacts. These artifacts are for side-by-side real-device acceptance testing before a tagged release; they are not a public release channel and expire automatically.

Use these binaries for Mac/Windows burn-in rather than installing a Rust toolchain on the target machine. Enrollment/state/startup must remain separate from any live v0.3 installation until the acceptance matrix and benchmark are complete.
## Remote POSIX controller/gateway burn-in

After a macOS/Linux device is enrolled against a private acceptance gateway, `scripts/burnin-remote-posix.sh` can drive persistence from the gateway host without relying on another remote-control product. Supply the gateway HTTP URL, device ID, and the target's Infra reconnect command through environment variables; no device-specific addresses or credentials belong in the repository.

The harness opens a PTY, starts a durable background job, restarts only the remote Infra controller, requires a newer connection generation, then requires **fresh** output from the same PTY and same job ID. If `INFRA_BURNIN_GATEWAY_RESTART_COMMAND` is supplied, it also restarts the gateway and requires fresh I/O from the same PTY afterward. This is the acceptance path used for the real macOS ARM64 side-by-side burn-in. After close/cancel, macOS acceptance must also verify that no `io.phloraxx.infra.worker.*` launchd jobs or PTY/job worker processes remain; the current self-cleaning launchd wrapper passes that check on the real Mac.

## Dashboard browser regression gate

`node scripts/test-dashboard-browser.cjs` runs an isolated fixture server and
Playwright Chromium. Install Playwright in a separate development environment and
make it resolvable through Node's normal module resolution (or `NODE_PATH`). This
adds no Node/browser dependency to installed agents or gateway users. The optional
`INFRA_CHROMIUM_EXECUTABLE` selects an existing Chromium binary, and
`INFRA_QA_SCREENSHOT` saves a mobile QA image.

GitHub Actions also runs this gate in a dedicated Dashboard browser job using
pinned Playwright 1.62.1 installed outside the workspace.

The gate covers device views, literal rendering of hostile-looking names/content,
file preview, session filters, same-ID terminal attachment across refresh,
input/resize/detach, no write replay after ambiguous failure, job cancellation,
mobile overflow, sign-out clearing and CSP errors. Run the real
`scripts/smoke-install-dashboard.sh` as well: it verifies the API and actual agent
PTY worker shared with MCP.


## Password and live-domain gates

Run `scripts/smoke-dashboard-password.sh` for real gateway coverage of Argon2id
owner login, bad credentials, rate limiting, session cookies, password rotation,
old-session revocation, origin rejection, HTTPS routing and public MCP isolation.
This is also an Ubuntu CI step. The browser fixture gate additionally tests the
signed-out screen and ensures the app stays hidden after a failed login.

An explicit live gate is available at `scripts/test-live-dashboard-browser.cjs`.
It requires `INFRA_QA_URL` (HTTPS), `INFRA_QA_CREDENTIALS_FILE` (a private JSON file
containing username/password), and optional `INFRA_QA_MIN_DEVICES`/screenshot path.
It reads credentials from the file instead of argv and redacts the password from
errors. It verifies real browser login, live online-device counts, secure cookie
attributes, logo/favicon, PTY input/refresh/close, cleanup and sign-out. Run only
against a deliberately selected authorized deployment; it opens a short-lived
`cat` terminal on an online device.
