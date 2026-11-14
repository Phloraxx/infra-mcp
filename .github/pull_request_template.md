## What changed

<!-- The behaviour being changed and why. Link the issue if there is one. -->

## Affected areas

- [ ] `crates/protocol`
- [ ] `crates/gateway`
- [ ] `crates/agent`
- [ ] `crates/platform`
- [ ] `crates/local-state`
- [ ] `crates/cli`
- [ ] `apps/dashboard`
- [ ] `apps/android-agent`
- [ ] installers / release scripts
- [ ] docs only

## Verification

<!-- Commands run, and what they proved. -->

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Wire and security impact

- [ ] No wire format change
- [ ] Wire format changed — protocol version bumped, `docs/PROTOCOL.md` updated
- [ ] No change to authentication, enrollment, signing or audit behaviour
- [ ] Authentication, enrollment, signing or audit behaviour changed — described below
- [ ] No new destructive operation
- [ ] New destructive operation — marked as such in tool metadata and API semantics

<!-- If any box above says "changed", explain the trust-boundary effect. -->

## Rollback

<!-- What happens to existing devices, state files and databases if this is reverted. -->
