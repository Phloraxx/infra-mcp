# Infra release manifest publisher key

The Ed25519 public key for Infra's versioned release manifest (not an OS binary signing certificate):

```text
DItK3Wlg98cSI7ptTNOd0oTq1I42paHDygEnEFc3eVs=
```

SHA-256 fingerprint of the 32-byte decoded public key:

```text
78a25d610bb35281b92b6f3c145ced4ba949f8dc96d8d94ced6152154e0422b1
```

The corresponding private signing seed is stored in the owner's macOS Keychain and in the encrypted GitHub Actions repository secret. Never place that private seed in releases, scripts, the repository or chat.

A valid Ed25519 manifest signature plus SHA-256 binary digest authenticates release bytes to this key only if the reader trusts the key independently. Downloading both the key and binaries from the same compromised location does not establish independent publisher trust. The first-install PowerShell/POSIX scripts are not yet independently authenticated. These release-candidate binaries are for explicit testing, not automated deployment.
