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
- This does not implement full TUF rollback/freeze protection. Old correctly signed
  manifests could be replayed if a release endpoint were compromised. Use an explicit
  version for controlled releases; design anti-rollback/version pinning before automatic updates.
- Keep the existing staged health-check and rollback functionality and test it on real
  macOS/Windows/Linux machines before enabling unattended rollouts.
