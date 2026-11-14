# Native updater: signed releases

`infra update` authenticates `infra-release.json` with an Ed25519 signature
(`infra-release.json.sig`) **before** staging any executable. The manifest binds
a release version, a monotonic release index and the SHA-256 digests of all
supported binaries. `scripts/install.sh` verifies the same manifest before it
executes a downloaded binary.

The publisher key, its fingerprint and every place it is embedded are recorded
in [RELEASE_PUBLISHER_KEY.md](RELEASE_PUBLISHER_KEY.md).

## What a release contains

| File | Purpose |
| --- | --- |
| `infra-<target>` / `infra-agent-<target>` | six platform pairs (Linux, macOS, Windows x64/ARM64) |
| `<asset>.sha256` | human-readable sidecar; not the trust root |
| `infra-release.json` | signed manifest: `version`, `source_commit`, `release_index`, `assets` |
| `infra-release.json.sig` | base64 Ed25519 signature over the exact manifest bytes |

## One-time owner provisioning (offline)

This is the only step that touches the real publisher key. Do it on a machine
you control, offline, and never paste the seed into a chat, an issue, a shell
history or a CI log.

1. Generate a 32-byte seed and derive the public key. The seed is written to a
   file with owner-only permissions; only the public key and fingerprint are
   printed.

   ```sh
   umask 077
   seed_file="$HOME/infra-publisher.seed"
   python3 - "$seed_file" <<'PY'
   import base64, os, sys
   with open(sys.argv[1], 'w', encoding='ascii') as handle:
       handle.write(base64.b64encode(os.urandom(32)).decode() + '\n')
   PY
   python3 - "$seed_file" <<'PY'
   import base64, hashlib, sys
   from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
   from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
   seed = base64.b64decode(open(sys.argv[1], encoding='ascii').read().strip())
   key = Ed25519PrivateKey.from_private_bytes(seed)
   public = key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
   print('public     ', base64.b64encode(public).decode())
   print('fingerprint', hashlib.sha256(public).hexdigest())
   PY
   ```

2. Store the seed in the owner's credential store (macOS Keychain, or an
   offline password manager) and delete the file:

   ```sh
   security add-generic-password -a infra -s infra-release-signing -w "$(cat "$seed_file")"
   rm -f "$seed_file"
   ```

3. Embed the **public** key in three places, in one commit:

   - `docs/RELEASE_PUBLISHER_KEY.md` — key and fingerprint.
   - `scripts/install.sh` — `RELEASE_PUBLIC_KEY_B64`, so the bootstrap verifies
     before exec.
   - GitHub repository variable `INFRA_RELEASE_PUBLIC_KEY_B64` — embedded into
     release builds and checked by the signing job.

4. Store the **private** seed only as the GitHub Actions repository secret
   `INFRA_RELEASE_SIGNING_KEY_B64` (base64 of the raw 32-byte seed). The signing
   job refuses to publish when the secret and the public key variable disagree,
   and it never prints either value.

5. Confirm the wiring without publishing anything:

   ```sh
   python3 scripts/test-release-signing.py     # fixtures, public test seed only
   python3 scripts/test-release-policy.py      # needs built debug binaries
   python3 scripts/test-install-manifest.py    # bootstrap verification
   ```

## Release index and anti-rollback

A valid old signature is not enough to serve an older release as current.

- The manifest carries `release_index`, a positive integer that must increase
  with every published release. The release workflow derives it from
  `git rev-list --count HEAD` on a full-history checkout, so it is monotonic on
  `main`; `scripts/sign-release.py` requires it explicitly and refuses zero.
- After a release has been fully verified and handed to activation, the CLI
  records the highest index it has seen in `<state-dir>/release-index.json`.
  The record only ever increases.
- Any manifest below that floor is refused with `update_rollback_blocked`
  before a single asset is requested. Manifests published before the counter
  existed have no index and are treated as `0`, so they can never follow an
  indexed release on the same device.
- An unreadable floor file fails closed instead of resetting to zero.
- `infra update --verify-only` runs the whole chain — pinned key, signature,
  version authorization, floor, asset digests, binary version pinning — and
  stops before installing anything. It never advances the floor.

This is not full TUF rollback/freeze protection. Same-version repair is still
allowed, the floor is per state directory, manually replacing the CLI can lower
its version floor, and there is no expiry or highest-ever-version claim outside
the recorded file.

## CI cycle with a test key

`.github/workflows/release.yml` runs the whole cycle without the publisher
credential:

- **producer** assembles the six platform pairs and signs three manifests —
  `current`, `next` and `old` — with either the owner key (tagged release) or a
  throwaway CI test key generated per run (`scripts/ci-test-key.py`). Neither
  key is printed, and the test key is never uploaded as an artifact.
- **verify** runs `scripts/verify-release-cycle.py` against exactly the bytes
  that would be published: the current release verifies, a device at the
  current index accepts the next release, an older signed release is refused as
  `update_rollback_blocked` before any asset is requested, a tampered manifest
  and a tampered asset are refused, and a requested-version mismatch is
  refused. No installed file, activation or floor changes.
- **release** publishes only after `verify` passes.

`.github/workflows/rust.yml` runs the same cycle on every push against locally
built binaries and the public deterministic test seed
(`scripts/fixture-release.py`), so the acceptance is continuous rather than
on-demand. SemVer downgrade refusal is covered by
`scripts/test-release-policy.py`.

Run the cycle locally against a candidate release directory:

```sh
python3 scripts/fixture-release.py --binaries target/debug \
  --directory /tmp/release --manifests /tmp/manifests > /tmp/public-key.txt
python3 scripts/verify-release-cycle.py --release-dir /tmp/release \
  --manifests /tmp/manifests --public-key-b64 "$(cat /tmp/public-key.txt)"
```

## Bootstrap trust

`scripts/install.sh` downloads `infra-release.json` and its signature, verifies
them against the pinned publisher key, checks the manifest version against the
requested tag, checks both asset digests, and only then executes the staged
CLI. The verifier is embedded in the script (byte-identical to
`scripts/verify-release-manifest.py`, enforced by
`scripts/test-install-manifest.py`) and uses the Python standard library only.

- A missing `python3` is a hard error, not a silent downgrade.
- `--allow-checksum-only` still exists and is **unsafe and temporary**: a
  checksum served from the same host as the binary is an integrity check, not
  publisher authentication. It prints that warning on every use.
- `install.ps1` (Windows) is still checksum-only. Do not claim first-install
  provenance for Windows until it verifies the signed manifest too.
- The bootstrap script itself is fetched over TLS from the canonical origin and
  is not independently authenticated. A compromised script host can still
  forge its own verification procedure; the signed manifest protects the
  release artifacts, not the script.

## First real signed update on a Linux agent (owner-controlled)

Do this deliberately, on one device, after the CI cycle is green. It is not
part of implementation work and must not be automated by an agent.

1. Confirm `docs/RELEASE_PUBLISHER_KEY.md` matches the repository variable and
   that the secret is the matching seed. The signing job fails closed otherwise.
2. Bump the workspace version deliberately, refresh `docs/RELEASE_CANDIDATE.md`
   with the real acceptance matrix, and commit.
3. Tag `v<package version>` and push the tag. Watch `producer` and `verify`;
   `release` publishes only after both pass.
4. On the Linux agent, preflight without changing anything:

   ```sh
   infra update --verify-only --version v<version>
   # verified release v<version> (release index N) for linux-aarch64; no files were changed
   ```

5. Apply it, then confirm the pair and the connection:

   ```sh
   infra update --version v<version>
   infra status --verify
   cat ~/.infra-mcp-v1/release-index.json   # records N
   ```

6. Prove the refusals on the same device: `infra update --version v<older>`
   must fail with `update_downgrade_blocked`, and serving the previous signed
   manifest must fail with `update_rollback_blocked`.
7. If the gateway binary changed, add the deployment audit row with the real
   binary sha256 to [DASHBOARD.md](DASHBOARD.md).

Only after that one device is healthy should a staged rollout to the rest of
the fleet be considered.

## Candidate release version and artifact contract

The staged native update path enforces these checks after signature
verification:

- Release tags/manifests use full SemVer, optionally prefixed with one `v`.
  Abbreviated/non-semantic tags are rejected. An explicit requested version must
  match the signed version, including build metadata.
- A release below the running CLI's SemVer precedence is `update_downgrade_blocked`.
  Prerelease ordering is numeric where SemVer requires it; build metadata does not
  raise the version floor. Same-version repair is allowed. There is no implicit
  downgrade override.
- A release below the recorded release index is `update_rollback_blocked`.
- Both downloaded components must report the exact signed package version,
  including build metadata, before any installed pair activation or update receipt
  begins. Mismatch is `update_binary_version_mismatch`; known staged copies are
  removed and installed config/identity are preserved.
- Missing native-target entries are rejected before requesting that binary.

This policy covers `infra update`. The explicit developer/bootstrap `activate`
path remains a separate trusted-source repair path; this does not claim a global
anti-rollback guarantee for manual file replacement, old scripts or old CLIs.
Internal transaction rollback restores its previous pair without a network update
and remains supported. Actual released-version/protocol rollback still needs
separate acceptance.

Release automation checks `v<package version>` against Cargo metadata before
building a tag, then checks each built native pair's component/version/activation
metadata before packaging. `scripts/sign-release.py` requires the exact 12 names
for Linux, macOS and Windows x64/ARM64 pairs, matching filename-bound checksum
sidecars, nonempty regular files and no unexpected publishable entries. Existing
manifest/signature outputs can be regenerated. Symlink inputs/outputs are rejected.
The signed manifest includes `source_commit`; the workflow supplies its full tagged
checked-out commit, including for annotated tags. This is a signed provenance claim, not a reproducible-build proof or
OS executable signature. Python 3.9+, Cargo and the existing `cryptography` signing
dependency are used; no production key is needed for the fixture tests.

The historical `v0.1.0-rc.1` tag used package version `0.1.0` and predates these
checks. Do not retag it or treat it as a conforming new candidate. Choose the next
release's package version deliberately before tagging, and refresh
`RELEASE_CANDIDATE.md` with the matching version and actual acceptance matrix; a prerelease tag must also
be the actual Cargo prerelease version. The bootstrap default stays gated until
compatible authenticated distribution and physical acceptance exist.

Reproducible checks (using test-owned keys/paths only):

```sh
cargo test --workspace --locked
python3 scripts/test-release-policy.py
python3 scripts/test-release-signing.py
python3 scripts/test-install-manifest.py
python3 scripts/release_contract.py --binaries target/debug --tag v0.1.0
```

`test-release-policy.py` needs built debug CLI/agent binaries and compiles the
explicit fixture signer example. The signing tests use a public deterministic test
seed, never the release owner's credential. Hosted Windows continues to test
same-version signed bad-agent rollback using the actual deferred handoff. Linux
runs the negative network policy and signing fixtures; macOS/Windows additionally
check built release metadata. Full six-target publishing is not invoked by PR CI.
