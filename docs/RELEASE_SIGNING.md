# Native updater: signed releases

The `infra update` command now authenticates `infra-release.json` with an Ed25519 signature
(`infra-release.json.sig`) **before** staging any executable. The manifest binds a
release version to SHA-256 digests of all supported binaries.

## One-time setup (release owner)

1. Generate a new Ed25519 signing key *in a trusted local environment* and store the
   **private key securely**. Do not commit it, print it in CI logs, or put it in an installer.
   The signing workflow consumes its raw 32-byte seed, base64 encoded.
2. Configure GitHub Actions repository secret `INFRA_RELEASE_SIGNING_KEY_B64` to that
   private seed and repository variable `INFRA_RELEASE_PUBLIC_KEY_B64` to the base64
   encoded 32-byte public key.
3. Before releasing, verify that the public key matches the private key. The signing
   script checks this and refuses to publish when they differ.
4. The tagged release workflow embeds the public key at **build time**, assembles six
   platform pairs, signs their manifest, and uploads both manifest and signature.
   Without keys it fails closed.
5. `infra update` verifies the manifest signature and each binary checksum using
   the pinned key. Builds with no embedded key refuse unattended updates; local
   acceptance can specify `--trusted-update-key BASE64`. This cannot override a
   pinned build key.

Key rotation requires a deliberately trusted new binary or an independently
authorized rotation mechanism. Simply changing a remote key or signature is not enough.

## Limits and remaining work

- The bootstrap `install.sh` and `install.ps1` paths **still** use hash sidecars
  and are not authenticated by the release signing trust root. Do not claim that
  first-install provenance is secure yet. Prefer a verified, signed native installer
  and signed bootstrap metadata before public distribution.
- Signed updates reject SemVer releases older than the running CLI. This is not
  full TUF rollback/freeze protection: same-version repair remains allowed, an old
  same-version artifact can still be replayed, and manually replacing the CLI can
  lower its version floor. No persistent highest-ever version or expiry is claimed.
- Keep the existing staged health-check and rollback functionality and test it on real
  macOS/Windows/Linux machines before enabling unattended rollouts.

## Published release candidate

On 2026-10-24 the owner provisioned the repository release signing secret and public key variable. GitHub Actions published [v0.1.0-rc.1](https://github.com/Phloraxx/infra-mcp/releases/tag/v0.1.0-rc.1) with all six platform CLI/agent pairs and a valid signed release manifest. The public key and fingerprint are recorded in [RELEASE_PUBLISHER_KEY.md](RELEASE_PUBLISHER_KEY.md); its private counterpart was retained in the owner's macOS Keychain. This does **not** authenticate the bootstrap scripts or represent a production-ready release.


## Candidate release version and artifact contract

The staged native update path now enforces these checks after signature verification:

- Release tags/manifests use full SemVer, optionally prefixed with one `v`.
  Abbreviated/non-semantic tags are rejected. An explicit requested version must
  match the signed version, including build metadata.
- A release below the running CLI's SemVer precedence is `update_downgrade_blocked`.
  Prerelease ordering is numeric where SemVer requires it; build metadata does not
  raise the version floor. Same-version repair is allowed. There is no implicit
  downgrade override.
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
release's package version deliberately before tagging; a prerelease tag must also
be the actual Cargo prerelease version. The bootstrap default stays gated until
compatible authenticated distribution and physical acceptance exist.

Reproducible checks (using test-owned keys/paths only):

```sh
cargo test --workspace --locked
python3 scripts/test-release-policy.py
python3 scripts/test-release-signing.py
python3 scripts/release_contract.py --binaries target/debug --tag v0.1.0
```

`test-release-policy.py` needs built debug CLI/agent binaries and compiles the
explicit fixture signer example. The signing tests use a public deterministic test
seed, never the release owner's credential. Hosted Windows continues to test
same-version signed bad-agent rollback using the actual deferred handoff. Linux
runs the negative network policy and signing fixtures; macOS/Windows additionally
check built release metadata. Full six-target publishing is not invoked by PR CI.
