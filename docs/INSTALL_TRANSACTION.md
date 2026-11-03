# Native installation transaction

2 November 2026 candidate source. Not deployed; not a consumer release.

Both bootstrap adapters stage/check both downloads and launch the staged native
CLI's `activate` operation. They no longer carry independent binary replacement,
rollback or cached-health logic. Normal reruns can use an existing activation-v1
CLI for native setup; explicit version requests bypass that fast path.

**A compatible new release must be published before these adapters are promoted.**
The older pinned `v0.1.0-rc.1` artifact predates `--build-info`/native activation;
it is rejected before installed files are changed. Development uses `--source-dir`
or `-SourceDir`. Download SHA-256 sidecars remain integrity checks; first-install
publisher authentication and OS signing remain separate gates. Signed update
version rules are documented in [RELEASE_SIGNING.md](RELEASE_SIGNING.md).

## Exclusion and recovery

Mutating native lifecycle commands hold nonblocking OS file locks for both the
installation and profile. A competing operation returns `installation_busy`.
The profile lock lives outside removable profile state; lock files are never
unlinked. Handles release locks on process exit, including a killed owner.
Canonical paths avoid separate locks through aliases. The binaries must live
outside the removable state tree. Read-only system package directories are not
certified by these per-user installer tests.

Implementation uses Rust's [File::try_lock](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock),
with cross-process crash/reacquisition tests. Locks coordinate participating
native versions; they cannot serialize arbitrary old scripts/manual file edits.

Before replacement, the engine validates the agent's bounded build-info response,
matching component/version and activation protocol. The staged CLI verifies its
own bytes match the candidate CLI. Both candidate images and prior-image backups
are copied, hashed and synced before an immutable UUID-bound journal is written.
Installed files are then replaced while the native locks remain held.

The journal records image hashes, profile binding and prior configuration metadata;
it never copies signing identity/key files. Setup failure restores the previous
pair/configuration. Failure to restore retains journal/backups and reports an
error. A complete first-install pair is retained if pairing merely remains
incomplete; identity/pending keys remain available for retry and no connection
success is claimed.

An interrupted swap is recovered by the next staged installer for the same
profile. Foreign profiles/unknown journal versions fail without restoration.
Unexpected image changes or damaged backups are preserved for inspection rather
than overwritten. A commit marker distinguishes completed activation from an
interrupted swap; cleanup removes the journal before its marker so interrupted
cleanup cannot turn a committed installation into a false rollback request.

These are recoverable two-file operations under exclusion, not a hardware-atomic
two-file rename. Power-loss/filesystem guarantees and adversarial local writers
are not certified. Native startup stop checks exact registered executable/profile
before stopping a running controller; durable workers retain their own lifetime.

## Update activation

The updater still verifies signed manifest/hash metadata before staging. It then
delegates to the candidate native CLI. Matching package versions are required
within the new pair, while upgrades may change the package version. Releases
without activation protocol 1 fail before installed changes. An expected prior
pair fingerprint prevents stale handoffs overwriting a different pair. Signed
updates reject versions older than the running CLI and require both candidate
build-info versions to match the manifest before activation; same-version repair
remains allowed. This does not impose a persistent version floor on manual repair
or change internal transaction rollback.

Unix waits for native activation. Windows cannot replace its currently executing
CLI reliably, so a small local PowerShell relay waits for that parent to exit,
runs the same locked native operation, records the result and cleans the source
directory. A bounded short PowerShell launcher calls local `Win32_Process.Create`
through CIM, which brokers the relay outside the MCP caller's Job Object and
without inherited capture pipes. No remote CIM connection, elevation verb, new
service/task, or job breakaway permission is used. The relay checks its user SID
and effective administrator status against its caller before native activation;
a mismatch fails closed and the receipt records `context_verified: false`.
The short launcher uses normal process-tree ownership; the old root-only runner
exception is removed. See Microsoft's [Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
and [process creation](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/create-method-in-class-win32-process).
Local CIM availability and permissions are now an explicit dependency; failure
returns an uncertain handoff with inspection guidance, never an elevated fallback.
The relay is UTF-8 with a BOM for Windows PowerShell 5.1 Unicode paths.

Direct Rust spawning with null stdio previously leaked capture handles. The
subsequent shell-verb relay passed direct invocation but died after MCP exec
returned. The caller-lifetime fixture exposed this second failure. This handoff
still uses the existing native swap/rollback state machine.

Windows returns **pending**, not completed. `infra status --json` adds
`update_status` for the latest request. Per-request receipts live under
`state/updates`; old completions cannot overwrite the newest request. A missing
owner or stale pending receipt is `outcome_unknown`, never automatic success or
automatic replay. Receipt history and relay logs are retained for diagnosis;
retention cleanup remains open. MCP/terminal helper-lifetime acceptance is now
exercised by the owned Windows fixture below; record its hosted result before
claiming that boundary passes.

Failure after activation attempts restoration and fresh signed verification of
the previous pair. A restored pre-readiness version may remain unverified; the
engine does not certify recovery merely because files were copied back.

## Bounded startup commands

The existing process-group/Windows Job Object runner is now shared by the agent
and CLI through `infra-platform`. Startup queries/actions use a 15-second command
budget plus the runner's two-second cleanup allowance; build-info uses five
seconds. No duplicate command-capture implementation or new daemon is added.
Each lifecycle can contain multiple bounded actions and readiness waits; this
does not promise a whole-machine deadline when filesystem/OS APIs themselves fail.

## Acceptance

Unit/owned-file fixtures cover cross-process lock release after kill, profile
removal while locked, interrupted first swap, damaged backup retry, committed
cleanup, absent candidates, foreign profiles, unrelated files and config/identity
preservation. Hosted Windows acceptance additionally holds a CLI file open to
force a real second-swap failure and drives a fixture-signed bad update through
the deferred helper to check both restored hashes, stable identity and fresh
signed operation. The captured caller must return while the receipt is still
pending, so finishing a background update before pipe EOF cannot mask a leaked
handle. Record hosted results separately from physical-device tests.

No live host, daily-use profile or excluded production gateway is used by these
tests. Physical Windows standard-user/login/reboot/sleep/network, permissions,
first-install trust, rollback across actual released versions and burn-in remain
release gates.


## MCP caller lifetime acceptance

The Windows-owned fixture runs the same signed bad-agent update three ways:
local CLI capture, gateway MCP `exec`, and gateway MCP `terminal`. It closes the
terminal and verifies its worker exits while activation remains pending. The
fixture binary delays only its second build-info probe to leave time for caller
cleanup before controller stop; production binaries have no test delay or mode.

Each attempt is issued once. An interrupted exec response may report uncertain
outcome and must not request automatic replay. An independent observer checks the
per-request receipt, detects a vanished relay, verifies failed-readiness rollback,
compares both original hashes and exact identity bytes, obtains fresh signed
connection evidence, and executes a new command through MCP after recovery.
These are transport/tool-path tests, not actual AI model or physical-PC acceptance.

The first hosted run exposed quoted-path corruption in Windows MCP exec before
the update started. Exec and job commands now preserve the intentional shell
program with `cmd /D /S /C` and one raw outer quote pair, following
[Rust's cmd.exe guidance](https://doc.rust-lang.org/stable/std/os/windows/process/trait.CommandExt.html#tymethod.raw_arg).
This applies only to user-requested shell programs; structured executable calls
still pass individual arguments. A Windows regression test checks a PowerShell
script path and argument containing spaces, a quoted literal ampersand, and an
intentional shell operator. Terminal command-builder quoting is separate from
this fix; the update fixture types its command into an already open shell.

The shared test client accepts only explicit loopback HTTP origins, ignores proxy
environment settings and refuses redirects. The fixture owner credential is passed
via an owned temporary file, never command-line arguments or committed content.
No live Infra connector, production endpoint or daily-use startup is involved.
