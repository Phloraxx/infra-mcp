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
- OAuth settings
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

This builds the v1 gateway, agent and `infra` CLI, starts an isolated loopback gateway, verifies embedded dashboard/bootstrap assets, creates an enrollment, runs `scripts/install.sh` in `--source-dir` development mode with startup intentionally disabled, verifies config and identity persistence, starts the installed agent without the enrollment code, then verifies status/doctor plus sessions/activity APIs. It removes all temporary state and does not register a real service or touch v0.3.

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
