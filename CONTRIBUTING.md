# Contributing to Infra MCP

Infra MCP is remote-control software. The gateway, device agent, protocol, installers and update channel are all security-sensitive, so contributions are reviewed against the trust model before they are reviewed against taste.

## Before changing code

Read these first:

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — how the gateway, agents and dashboard fit together
- [`docs/PROTOCOL.md`](docs/PROTOCOL.md) — the versioned wire contract
- [`docs/SECURITY_MODEL.md`](docs/SECURITY_MODEL.md) — threat model and trust boundaries
- [`docs/AGENT_CLIENT_GUIDE.md`](docs/AGENT_CLIENT_GUIDE.md) — tool contracts visible to MCP clients
- [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) — workflow and workspace boundaries

## Invariants

These are not style preferences. A change that breaks one of them is a protocol or security change, not a feature.

- Device identity is an Ed25519 key held on the device. Private keys never leave it, and every connection re-runs challenge-response.
- A captured signature is bound to one connection. Replay must fail.
- Enrollment codes are random, short-lived, hashed at rest, single-use and owner-approved.
- The gateway authenticates MCP clients and devices as separate principals. Never share one credential path between them.
- Agents run as the signed-in user. Anything requiring elevation belongs in the optional privileged helper, never in the normal agent.
- Audit records store action, outcome and request IDs — never stdout, secrets or file contents.
- Releases ship a signed Ed25519 manifest with SHA-256 sidecars. The updater refuses unverified artifacts.
- Destructive operations must be distinguishable from read-only and reversible ones in both tool metadata and API semantics.
- Wire changes require a protocol version bump. Do not silently reinterpret an existing frame.

## Development setup

Rust stable, per the workspace `rust-version`. The dashboard has no build step; it is served as static assets from the gateway.

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the matrix on Ubuntu (x64 and ARM64) and macOS on every push, plus a headless dashboard suite and an Android job. The full Windows suite runs on release tags.

## Pull requests

Keep PRs focused and keep `main` buildable. Use a feature branch per concern rather than one large rewrite.

Include in the description:

- the behaviour being changed and why;
- which crates or apps are affected;
- tests added or updated;
- whether wire format, enrollment, signing, audit or update behaviour changed;
- any migration or rollback implication.

Commit messages follow Conventional Commits with a body that explains the reasoning, not just the diff. `feat:`, `fix:`, `test:`, `docs:`, `chore:`, `refactor:`, `ci:`.

## What not to send

- credentials, private keys, tokens, `.env` files, or captured session material;
- real device names, hostnames, IP addresses or terminal output from a live machine;
- a patch that disables an authentication, signing or audit check "temporarily";
- dependency bumps that also change behaviour (split them).

## Reporting a security issue

Do not open a public issue. Follow [`SECURITY.md`](SECURITY.md).

## Questions and design proposals

Large protocol, privilege or deployment ideas should start in [Discussions](https://github.com/Phloraxx/infra-mcp/discussions) before implementation.
