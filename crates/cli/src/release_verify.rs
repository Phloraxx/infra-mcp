//! Update authenticity is enforced before replacing either executable.
//! Release signing keys are provisioned explicitly at build-time or by the owner.
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    pub assets: HashMap<String, String>,
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
