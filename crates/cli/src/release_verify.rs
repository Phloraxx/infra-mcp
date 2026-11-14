//! Update authenticity is enforced before replacing either executable.
//! Release signing keys are provisioned explicitly at build-time or by the owner.
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    pub assets: HashMap<String, String>,
    /// Monotonic release counter signed into the manifest. Manifests published
    /// before the counter existed have none and are treated as index 0.
    #[serde(default)]
    pub release_index: Option<u64>,
}

/// The highest signed release index this device has verified and installed.
/// It only ever increases, so a replayed older release cannot be served as
/// current even when its signature is still valid.
#[derive(Debug, Serialize, Deserialize)]
struct ReleaseFloor {
    release_index: u64,
    version: String,
    recorded_unix_ms: u64,
}

fn floor_path(state: &Path) -> std::path::PathBuf {
    state.join("release-index.json")
}

/// A missing floor means this device has never installed an indexed release.
/// An unreadable floor fails closed instead of silently resetting to zero.
pub fn read_release_floor(state: &Path) -> Result<u64> {
    let path = floor_path(state);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("release_index_floor_unreadable: {}", path.display()));
        }
    };
    let floor: ReleaseFloor = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "release_index_floor_unreadable: {} is not a valid release index record",
            path.display()
        )
    })?;
    Ok(floor.release_index)
}

/// Records the floor after a release has been fully verified. The value never
/// decreases, so a stale or replayed record cannot lower it.
pub fn record_release_floor(state: &Path, release_index: u64, version: &str) -> Result<u64> {
    let current = read_release_floor(state)?;
    let release_index = current.max(release_index);
    let value = ReleaseFloor {
        release_index,
        version: version.to_owned(),
        recorded_unix_ms: infra_local_state::now_ms(),
    };
    let path = floor_path(state);
    let temporary = state.join(".release-index.json.tmp");
    std::fs::create_dir_all(state)?;
    std::fs::write(&temporary, serde_json::to_vec(&value)?)
        .with_context(|| format!("write release index floor {}", temporary.display()))?;
    std::fs::rename(&temporary, &path)
        .with_context(|| format!("record release index floor {}", path.display()))?;
    Ok(release_index)
}

/// A valid old signature does not authorize serving an older release as
/// current. Manifests without a counter are index 0 and can therefore never
/// follow an indexed release on the same device.
pub fn authorize_release_index(manifest: Option<u64>, floor: u64) -> Result<u64> {
    let release_index = manifest.unwrap_or(0);
    if release_index < floor {
        bail!(
            "update_rollback_blocked: signed release index {release_index} is below the recorded floor {floor}; an older signed release cannot be installed as current"
        );
    }
    Ok(release_index)
}

fn release_version(value: &str) -> Result<Version> {
    Version::parse(value.strip_prefix('v').unwrap_or(value))
        .with_context(|| format!("invalid_release_version: expected SemVer, received {value:?}"))
}

/// A valid old signature does not authorize a downgrade of this running CLI.
/// Same-version repair is allowed; build metadata does not raise precedence.
pub fn authorize_update_version(release: &str, current: &str, requested: &str) -> Result<String> {
    let release = release_version(release)?;
    let current = Version::parse(current).context("invalid running CLI package version")?;
    if requested != "latest" && release != release_version(requested)? {
        bail!(
            "update_version_mismatch: signed release {release} does not match requested {requested}"
        );
    }
    if release.cmp_precedence(&current).is_lt() {
        bail!(
            "update_downgrade_blocked: release {release} is older than running CLI {current}; automatic downgrade is not supported"
        );
    }
    Ok(release.to_string())
}

pub fn verify_binary_versions(release: &str, cli: &str, agent: &str) -> Result<()> {
    let expected = release_version(release)?;
    for (component, actual) in [("infra", cli), ("infra-agent", agent)] {
        let actual = Version::parse(actual).with_context(|| {
            format!("update_binary_version_mismatch: invalid {component} package version")
        })?;
        if actual != expected {
            bail!(
                "update_binary_version_mismatch: signed release {expected}, {component} reports {actual}; installed pair unchanged"
            );
        }
    }
    Ok(())
}

pub fn verify_manifest(
    manifest_bytes: &[u8],
    signature_base64: &str,
    trusted_public_key_base64: &str,
) -> Result<ReleaseManifest> {
    let public_bytes = STANDARD
        .decode(trusted_public_key_base64.trim())
        .context("trusted release public key must be base64")?;
    let key: [u8; 32] = public_bytes
        .try_into()
        .map_err(|_| anyhow!("trusted Ed25519 public key must be 32 bytes"))?;
    let key = VerifyingKey::from_bytes(&key).context("invalid Ed25519 release public key")?;
    let sig_bytes = STANDARD
        .decode(signature_base64.trim())
        .context("manifest signature must be base64")?;
    let signature =
        Signature::from_slice(&sig_bytes).context("invalid Ed25519 manifest signature")?;
    key.verify_strict(manifest_bytes, &signature)
        .map_err(|_| anyhow!("release manifest signature verification failed"))?;
    let manifest: ReleaseManifest =
        serde_json::from_slice(manifest_bytes).context("signed manifest is invalid JSON")?;
    if manifest.version.is_empty() || manifest.assets.is_empty() {
        bail!("signed release manifest is missing version/assets");
    }
    if manifest.release_index == Some(0) {
        bail!("signed release manifest has an invalid release index 0");
    }
    for (name, digest) in &manifest.assets {
        if name.is_empty() || digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("signed manifest contains invalid asset name or SHA-256");
        }
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    #[test]
    fn update_precedence_rejects_replay_and_handles_prereleases_and_build_metadata() {
        for (target, current) in [
            ("v1.9.9", "2.0.0"),
            ("v2.0.0-rc.9", "2.0.0"),
            ("v2.0.0-rc.2", "2.0.0-rc.10"),
        ] {
            assert!(
                authorize_update_version(target, current, "latest")
                    .unwrap_err()
                    .to_string()
                    .starts_with("update_downgrade_blocked:")
            );
        }
        for (target, current) in [
            ("v2.0.0", "2.0.0-rc.10"),
            ("v2.0.0-rc.10", "2.0.0-rc.2"),
            ("v2.0.0+old", "2.0.0+new"),
            ("v2.0.0", "2.0.0"),
        ] {
            assert!(authorize_update_version(target, current, "latest").is_ok());
        }
        assert!(authorize_update_version("v2.0.0", "1.0.0", "2.0.0").is_ok());
        assert!(authorize_update_version("v2.0.0+one", "1.0.0", "v2.0.0+two").is_err());
        for invalid in [
            "v-fixture",
            "v1",
            "01.0.0",
            "v1.0.0-01",
            " 1.0.0",
            "vv1.0.0",
        ] {
            assert!(authorize_update_version(invalid, "1.0.0", "latest").is_err());
        }
    }
    #[test]
    fn both_binaries_must_match_the_signed_version_including_build_metadata() {
        assert!(verify_binary_versions("v2.0.0+one", "2.0.0+one", "2.0.0+one").is_ok());
        assert!(verify_binary_versions("v2.0.0", "1.0.0", "1.0.0").is_err());
        assert!(verify_binary_versions("v2.0.0", "2.0.0", "2.0.1").is_err());
        assert!(verify_binary_versions("v2.0.0+one", "2.0.0+two", "2.0.0+one").is_err());
    }
    #[test]
    fn signatures_pin_exact_manifest_bytes() {
        let signer = SigningKey::from_bytes(&[11u8; 32]);
        let json = br#"{"version":"v1","assets":{"infra-linux-aarch64":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#;
        let sig = STANDARD.encode(signer.sign(json).to_bytes());
        let pubkey = STANDARD.encode(signer.verifying_key().to_bytes());
        let manifest = verify_manifest(json, &sig, &pubkey).unwrap();
        assert_eq!(manifest.version, "v1");
        assert_eq!(manifest.release_index, None);
        assert!(verify_manifest(b"{}", &sig, &pubkey).is_err());
        assert!(verify_manifest(json, &sig, &STANDARD.encode([9u8; 32])).is_err());
        assert!(verify_manifest(json, "invalid", &pubkey).is_err());
    }
    #[test]
    fn a_signed_release_index_is_parsed_and_zero_is_rejected() {
        let signer = SigningKey::from_bytes(&[12u8; 32]);
        let pubkey = STANDARD.encode(signer.verifying_key().to_bytes());
        let sign = |json: &[u8]| STANDARD.encode(signer.sign(json).to_bytes());
        let indexed = br#"{"version":"v1","release_index":7,"assets":{"infra-linux-aarch64":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#;
        assert_eq!(
            verify_manifest(indexed, &sign(indexed), &pubkey)
                .unwrap()
                .release_index,
            Some(7)
        );
        let zero = br#"{"version":"v1","release_index":0,"assets":{"infra-linux-aarch64":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#;
        assert!(
            verify_manifest(zero, &sign(zero), &pubkey)
                .unwrap_err()
                .to_string()
                .contains("invalid release index 0")
        );
    }
    #[test]
    fn an_older_signed_release_index_cannot_be_served_as_current() {
        assert_eq!(authorize_release_index(None, 0).unwrap(), 0);
        assert_eq!(authorize_release_index(Some(3), 0).unwrap(), 3);
        assert_eq!(authorize_release_index(Some(5), 5).unwrap(), 5);
        assert_eq!(authorize_release_index(Some(6), 5).unwrap(), 6);
        for (manifest, floor) in [(Some(4), 5), (None, 1), (Some(0), 1)] {
            assert!(
                authorize_release_index(manifest, floor)
                    .unwrap_err()
                    .to_string()
                    .starts_with("update_rollback_blocked:")
            );
        }
    }
    #[test]
    fn the_recorded_floor_only_increases_and_fails_closed_when_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        assert_eq!(read_release_floor(state).unwrap(), 0);
        assert_eq!(record_release_floor(state, 4, "v1.0.0").unwrap(), 4);
        assert_eq!(read_release_floor(state).unwrap(), 4);
        assert_eq!(record_release_floor(state, 2, "v0.9.0").unwrap(), 4);
        assert_eq!(read_release_floor(state).unwrap(), 4);
        assert_eq!(record_release_floor(state, 9, "v1.1.0").unwrap(), 9);
        assert_eq!(read_release_floor(state).unwrap(), 9);
        assert!(!state.join(".release-index.json.tmp").exists());
        std::fs::write(state.join("release-index.json"), b"not json").unwrap();
        assert!(
            read_release_floor(state)
                .unwrap_err()
                .to_string()
                .contains("release_index_floor_unreadable")
        );
    }
}
