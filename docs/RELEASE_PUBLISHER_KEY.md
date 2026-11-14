# Infra release manifest publisher key

The Ed25519 public key for Infra's versioned release manifest (not an OS binary
signing certificate):

```text
DItK3Wlg98cSI7ptTNOd0oTq1I42paHDygEnEFc3eVs=
```

SHA-256 fingerprint of the 32-byte decoded public key:

```text
78a25d610bb35281b92b6f3c145ced4ba949f8dc96d8d94ced6152154e0422b1
```

Check any copy of the key against that fingerprint before trusting it:

```sh
printf '%s' 'DItK3Wlg98cSI7ptTNOd0oTq1I42paHDygEnEFc3eVs=' |
  base64 -d | sha256sum
```

## What this key authorizes

- `infra-release.json` — the signed manifest binding a release version, a
  monotonic release index and the SHA-256 of every published binary.
- `infra update` — refuses to stage any executable unless the manifest verifies
  against this key (pinned at build time, or supplied explicitly for local
  acceptance).
- `scripts/install.sh` — refuses to execute a downloaded binary unless the
  manifest verifies against this key before exec.

It does **not** authenticate the bootstrap script itself, an OS package, or a
manually copied binary. A reader must trust the key independently: downloading
both the key and the binaries from the same compromised host proves nothing.

## Where the key lives

| Copy | Location | Purpose |
| --- | --- | --- |
| Public | this file | human reference and fingerprint |
| Public | `scripts/install.sh` (`RELEASE_PUBLIC_KEY_B64`) | bootstrap verification before exec |
| Public | GitHub repository variable `INFRA_RELEASE_PUBLIC_KEY_B64` | embedded into release builds and checked by the signing job |
| Private | owner's offline environment and macOS Keychain | signing |
| Private | GitHub Actions secret `INFRA_RELEASE_SIGNING_KEY_B64` | tagged-release signing only |

The private seed is never committed, printed, logged, uploaded as an artifact
or placed in a release. CI generates a throwaway test key for every non-tagged
run, so the full cycle is exercised without the publisher credential.

## Rotation

Rotation requires a deliberately trusted new binary or an independently
authorized rotation mechanism; changing a remote key or signature is not
enough. To rotate:

1. Generate the new key offline (see [RELEASE_SIGNING.md](RELEASE_SIGNING.md)).
2. Update this file, the fingerprint, `scripts/install.sh` and the repository
   variable in one commit, and record why in the commit message.
3. Replace the repository secret.
4. Publish one release signed with the new key and verify it with
   `infra update --verify-only` on a device that already trusts the old key
   before any fleet update.

## History

On 2026-10-24 the owner provisioned the repository release signing secret and
public key variable and published
[v0.1.0-rc.1](https://github.com/Phloraxx/infra-mcp/releases/tag/v0.1.0-rc.1)
with all six platform CLI/agent pairs and a valid signed release manifest. That
tag predates the release-index counter and the bootstrap manifest check; it is
not a conforming new candidate and must not be retagged.
