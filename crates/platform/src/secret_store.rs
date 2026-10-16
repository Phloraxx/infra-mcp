use anyhow::{Context, anyhow};
use keyring::Entry;

const DEVICE_IDENTITY_SERVICE: &str = "io.phloraxx.infra.device-identity";

/// Store a device signing secret in the platform-native credential store.
///
/// keyring 4's v1 backend maps this to Keychain Services on macOS, Windows Credential Manager
/// on Windows, and Secret Service on Linux/*nix. Callers decide whether an unavailable store may
/// fall back to a private file; this module never silently does so.
pub fn put_device_identity(key_id: &str, secret: &[u8]) -> anyhow::Result<()> {
    validate_key_id(key_id)?;
    let entry = Entry::new(DEVICE_IDENTITY_SERVICE, key_id)
        .map_err(|error| anyhow!("initialize native credential entry: {error}"))?;
    entry
        .set_secret(secret)
        .map_err(|error| anyhow!("store device identity in native credential store: {error}"))
}

pub fn get_device_identity(key_id: &str) -> anyhow::Result<Vec<u8>> {
    validate_key_id(key_id)?;
    let entry = Entry::new(DEVICE_IDENTITY_SERVICE, key_id)
        .map_err(|error| anyhow!("initialize native credential entry: {error}"))?;
    entry
        .get_secret()
        .map_err(|error| anyhow!("read device identity from native credential store: {error}"))
}

pub fn delete_device_identity(key_id: &str) -> anyhow::Result<()> {
    validate_key_id(key_id)?;
    let entry = Entry::new(DEVICE_IDENTITY_SERVICE, key_id)
        .map_err(|error| anyhow!("initialize native credential entry: {error}"))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(anyhow!(
            "delete device identity from native credential store: {error}"
        )),
    }
}

pub fn native_store_status() -> anyhow::Result<()> {
    Entry::store_status()
        .as_ref()
        .map_err(|error| anyhow!("native credential store unavailable: {error}"))
        .context("initialize native credential store")
        .map(|_| ())
}

fn validate_key_id(key_id: &str) -> anyhow::Result<()> {
    if key_id.is_empty() || key_id.len() > 128 {
        return Err(anyhow!("credential key id must be 1..128 bytes"));
    }
    if !key_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(anyhow!("credential key id contains unsupported characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_ids_are_bounded_and_path_independent() {
        assert!(validate_key_id("device.1234-abcd_test").is_ok());
        assert!(validate_key_id("").is_err());
        assert!(validate_key_id("../secret").is_err());
        assert!(validate_key_id(&"x".repeat(129)).is_err());
    }
}
