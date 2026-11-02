//! Update authenticity is enforced before replacing either executable.
//! Release signing keys are provisioned explicitly at build-time or by the owner.
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    pub assets: HashMap<String, String>,
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
        assert!(verify_manifest(b"{}", &sig, &pubkey).is_err());
        assert!(verify_manifest(json, &sig, &STANDARD.encode([9u8; 32])).is_err());
        assert!(verify_manifest(json, "invalid", &pubkey).is_err());
    }
}
