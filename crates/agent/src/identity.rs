use anyhow::{Context, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use infra_platform::secret_store;
use infra_protocol::{DeviceHello, signed_auth_message};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tracing::warn;
use url::Url;
use uuid::Uuid;

const IDENTITY_VERSION: u8 = 2;

#[derive(Debug, Clone)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub device_name: String,
    storage: SecretStorage,
    signing_key: SigningKey,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SecretStorage {
    NativeKeyring,
    FileFallback,
}

#[derive(Debug, Serialize, Deserialize)]
struct IdentityFileV2 {
    version: u8,
    device_id: String,
    device_name: String,
    key_id: String,
    storage: SecretStorage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingIdentityV2 {
    version: u8,
    request_id: String,
    device_name: String,
    key_id: String,
    storage: SecretStorage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyIdentityFile {
    version: u8,
    device_id: String,
    device_name: String,
    secret_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyPendingIdentity {
    version: u8,
    request_id: String,
    device_name: String,
    secret_key: String,
}

#[derive(Debug, Serialize)]
struct EnrollRequest<'a> {
    code: &'a str,
    request_id: &'a str,
    device_name: &'a str,
    public_key: String,
    platform: &'a str,
    arch: &'a str,
    agent_version: &'a str,
}

#[derive(Debug, Deserialize)]
struct EnrollResponse {
    device_id: String,
    device_name: String,
}

impl DeviceIdentity {
    pub fn storage_label(&self) -> &'static str {
        match self.storage {
            SecretStorage::NativeKeyring => "native_keyring",
            SecretStorage::FileFallback => "file_fallback",
        }
    }

    #[cfg(test)]
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn sign_hello(&self, nonce: &str, gateway_epoch: Uuid, hello: &DeviceHello) -> String {
        let message = signed_auth_message(nonce, gateway_epoch, hello);
        URL_SAFE_NO_PAD.encode(self.signing_key.sign(&message).to_bytes())
    }
}

pub fn default_state_dir() -> PathBuf {
    infra_local_state::default_state_dir()
}

pub fn ensure_state_dir(path: &Path) -> anyhow::Result<()> {
    infra_local_state::ensure_state_dir(path)
}

pub fn load(path: &Path) -> anyhow::Result<Option<DeviceIdentity>> {
    let file = path.join("identity.json");
    let bytes = match fs::read(&file) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let version = json_version(&bytes, "identity.json")?;
    let stored = match version {
        1 => migrate_legacy_identity(&file, serde_json::from_slice(&bytes)?)?,
        IDENTITY_VERSION => {
            serde_json::from_slice::<IdentityFileV2>(&bytes).context("decode identity.json")?
        }
        other => return Err(anyhow!("unsupported identity file version {other}")),
    };
    let secret =
        load_protected_secret(stored.storage, &stored.key_id, stored.secret_key.as_deref())?;
    Ok(Some(DeviceIdentity {
        device_id: stored.device_id,
        device_name: stored.device_name,
        storage: stored.storage,
        signing_key: SigningKey::from_bytes(&secret),
    }))
}

pub async fn load_or_enroll(
    state_dir: &Path,
    gateway_ws_url: &str,
    enrollment_code: Option<&str>,
    requested_name: &str,
    platform: &str,
    arch: &str,
    agent_version: &str,
) -> anyhow::Result<Option<DeviceIdentity>> {
    ensure_state_dir(state_dir)?;
    if let Some(identity) = load(state_dir)? {
        return Ok(Some(identity));
    }
    let Some(code) = enrollment_code.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };

    let pending_path = state_dir.join("pending-enrollment.json");
    let pending = load_or_create_pending(&pending_path, requested_name)?;
    let secret = load_protected_secret(
        pending.storage,
        &pending.key_id,
        pending.secret_key.as_deref(),
    )?;
    let signing_key = SigningKey::from_bytes(&secret);
    let public_key = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes());
    let enroll_url = enrollment_url(gateway_ws_url)?;
    let response = reqwest::Client::new()
        .post(enroll_url)
        .json(&EnrollRequest {
            code,
            request_id: &pending.request_id,
            device_name: &pending.device_name,
            public_key,
            platform,
            arch,
            agent_version,
        })
        .send()
        .await
        .context("send enrollment request")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("enrollment failed ({status}): {body}"));
    }
    let enrolled: EnrollResponse = response
        .json()
        .await
        .context("decode enrollment response")?;
    let stored = IdentityFileV2 {
        version: IDENTITY_VERSION,
        device_id: enrolled.device_id.clone(),
        device_name: enrolled.device_name.clone(),
        key_id: pending.key_id,
        storage: pending.storage,
        secret_key: pending.secret_key,
    };
    write_secret_json(&state_dir.join("identity.json"), &stored)?;
    let _ = fs::remove_file(pending_path);
    Ok(Some(DeviceIdentity {
        device_id: enrolled.device_id,
        device_name: enrolled.device_name,
        storage: stored.storage,
        signing_key,
    }))
}

#[derive(Debug, Deserialize)]
struct PairResponse {
    state: String,
    verification_code: String,
    expires_at_unix_ms: u64,
    device_id: Option<String>,
    device_name: String,
}

#[derive(Debug, Serialize)]
struct PairCreateRequest<'a> {
    request_id: &'a str,
    device_name: &'a str,
    public_key: &'a str,
    platform: &'a str,
    arch: &'a str,
    agent_version: &'a str,
    signature: String,
}

#[derive(Debug, Serialize)]
struct PairPollRequest<'a> {
    request_id: &'a str,
    public_key: &'a str,
    signature: String,
}

// Device-side pairing shares the original pending Ed25519 key + request ID with
// legacy enrollment, so a failed network request never creates a second key.
pub async fn pair_with_owner(
    state_dir: &Path,
    gateway_ws_url: &str,
    detected_name: &str,
    platform: &str,
    arch: &str,
    version: &str,
) -> anyhow::Result<DeviceIdentity> {
    ensure_state_dir(state_dir)?;
    if let Some(identity) = load(state_dir)? {
        println!("Infra is already paired as {}.", identity.device_name);
        return Ok(identity);
    }
    let pending_path = state_dir.join("pending-enrollment.json");
    let pending = load_or_create_pending(&pending_path, detected_name)?;
    let secret = load_protected_secret(
        pending.storage,
        &pending.key_id,
        pending.secret_key.as_deref(),
    )?;
    let signing_key = SigningKey::from_bytes(&secret);
    let key = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes());
    let mut request_url = enrollment_url(gateway_ws_url)?;
    request_url.set_path("/pair/request");
    let mut status_url = request_url.clone();
    status_url.set_path("/pair/status");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()?;
    let message = format!(
        "infra-pair-request-v1\n{}\n{}\n{}\n{}\n{}\n{}",
        pending.request_id, key, pending.device_name, platform, arch, version,
    );
    let create = PairCreateRequest {
        request_id: &pending.request_id,
        device_name: &pending.device_name,
        public_key: &key,
        platform,
        arch,
        agent_version: version,
        signature: URL_SAFE_NO_PAD.encode(signing_key.sign(message.as_bytes()).to_bytes()),
    };
    let proof = format!("infra-pair-status-v1\n{}", pending.request_id);
    let poll = PairPollRequest {
        request_id: &pending.request_id,
        public_key: &key,
        signature: URL_SAFE_NO_PAD.encode(signing_key.sign(proof.as_bytes()).to_bytes()),
    };

    // The process may exit and be rerun safely; the protected pending key remains.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10 * 60);
    let mut last_code = String::new();
    loop {
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("pairing approval timed out; run Infra setup again to resume");
        }
        let response = client
            .post(request_url.clone())
            .json(&create)
            .send()
            .await
            .context("request owner pairing (check DNS and TLS)")?;
        if !response.status().is_success() {
            anyhow::bail!(
                "pairing request rejected (HTTP {}); your previous identity was preserved",
                response.status()
            );
        }
        let mut view: PairResponse = response.json().await.context("decode pairing request")?;
        if view.verification_code != last_code {
            let dashboard = request_url.origin().ascii_serialization();
            println!(
                "Infra wants to connect this computer: {}",
                pending.device_name
            );
            println!(
                "Open {} and approve the matching code: {}",
                dashboard, view.verification_code
            );
            last_code = view.verification_code.clone();
        }
        while view.state == "pending" && tokio::time::Instant::now() < deadline {
            if view.expires_at_unix_ms <= infra_local_state::now_ms() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let response = client
                .post(status_url.clone())
                .json(&poll)
                .send()
                .await
                .context("check pairing approval (retry setup if network is offline)")?;
            if !response.status().is_success() {
                anyhow::bail!(
                    "pairing status unavailable (HTTP {}); retry setup to resume",
                    response.status()
                );
            }
            view = response.json().await.context("decode pairing status")?;
        }
        if view.state == "approved" {
            let device_id = view
                .device_id
                .ok_or_else(|| anyhow!("approved pairing has no device ID"))?;
            let stored = IdentityFileV2 {
                version: IDENTITY_VERSION,
                device_id: device_id.clone(),
                device_name: view.device_name.clone(),
                key_id: pending.key_id,
                storage: pending.storage,
                secret_key: pending.secret_key,
            };
            write_secret_json(&state_dir.join("identity.json"), &stored)?;
            let _ = fs::remove_file(pending_path);
            println!("Pairing approved. Checking connection next.");
            return Ok(DeviceIdentity {
                device_id,
                device_name: stored.device_name,
                storage: stored.storage,
                signing_key,
            });
        }
        if view.state != "expired" && view.state != "pending" {
            anyhow::bail!("pairing returned an unexpected state; previous identity preserved");
        }
    }
}

fn load_or_create_pending(path: &Path, requested_name: &str) -> anyhow::Result<PendingIdentityV2> {
    match fs::read(path) {
        Ok(bytes) => match json_version(&bytes, "pending-enrollment.json")? {
            1 => {
                let legacy: LegacyPendingIdentity = serde_json::from_slice(&bytes)
                    .context("decode legacy pending enrollment identity")?;
                let secret = decode_secret(&legacy.secret_key)?;
                let protected = protect_secret(&secret);
                let pending = PendingIdentityV2 {
                    version: IDENTITY_VERSION,
                    request_id: legacy.request_id,
                    device_name: legacy.device_name,
                    key_id: protected.key_id,
                    storage: protected.storage,
                    secret_key: protected.secret_key,
                };
                if let Err(error) = write_secret_json(path, &pending) {
                    cleanup_native_secret(pending.storage, &pending.key_id);
                    return Err(error);
                }
                Ok(pending)
            }
            IDENTITY_VERSION => {
                serde_json::from_slice(&bytes).context("decode pending enrollment identity")
            }
            other => Err(anyhow!("unsupported pending identity version {other}")),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let mut secret = [0_u8; 32];
            getrandom::fill(&mut secret).map_err(|_| anyhow!("OS random source unavailable"))?;
            let protected = protect_secret(&secret);
            let pending = PendingIdentityV2 {
                version: IDENTITY_VERSION,
                request_id: Uuid::new_v4().to_string(),
                device_name: requested_name.to_string(),
                key_id: protected.key_id,
                storage: protected.storage,
                secret_key: protected.secret_key,
            };
            write_secret_json(path, &pending)?;
            Ok(pending)
        }
        Err(err) => Err(err.into()),
    }
}

struct ProtectedSecret {
    key_id: String,
    storage: SecretStorage,
    secret_key: Option<String>,
}

fn protect_secret(secret: &[u8; 32]) -> ProtectedSecret {
    let key_id = Uuid::new_v4().to_string();
    match secret_store::put_device_identity(&key_id, secret).and_then(|_| {
        let read_back = secret_store::get_device_identity(&key_id)?;
        if read_back.as_slice() != secret {
            return Err(anyhow!("native credential store read-back mismatch"));
        }
        Ok(())
    }) {
        Ok(()) => ProtectedSecret {
            key_id,
            storage: SecretStorage::NativeKeyring,
            secret_key: None,
        },
        Err(error) => {
            // `key_id` is freshly generated, so a failed verification can only leave behind
            // the credential written by this attempt. Remove it before falling back to disk.
            let _ = secret_store::delete_device_identity(&key_id);
            warn!(%error, "native credential store unavailable; using private file fallback");
            ProtectedSecret {
                key_id,
                storage: SecretStorage::FileFallback,
                secret_key: Some(URL_SAFE_NO_PAD.encode(secret)),
            }
        }
    }
}

fn load_protected_secret(
    storage: SecretStorage,
    key_id: &str,
    file_secret: Option<&str>,
) -> anyhow::Result<[u8; 32]> {
    match storage {
        SecretStorage::NativeKeyring => {
            let bytes = secret_store::get_device_identity(key_id)
                .context("load device signing key from native credential store")?;
            bytes
                .try_into()
                .map_err(|_| anyhow!("native device signing key has invalid length"))
        }
        SecretStorage::FileFallback => decode_secret(
            file_secret.ok_or_else(|| anyhow!("file-backed identity secret is missing"))?,
        ),
    }
}

fn migrate_legacy_identity(
    path: &Path,
    legacy: LegacyIdentityFile,
) -> anyhow::Result<IdentityFileV2> {
    if legacy.version != 1 {
        return Err(anyhow!(
            "unsupported legacy identity version {}",
            legacy.version
        ));
    }
    let secret = decode_secret(&legacy.secret_key)?;
    let protected = protect_secret(&secret);
    let migrated = IdentityFileV2 {
        version: IDENTITY_VERSION,
        device_id: legacy.device_id,
        device_name: legacy.device_name,
        key_id: protected.key_id,
        storage: protected.storage,
        secret_key: protected.secret_key,
    };
    if let Err(error) = write_secret_json(path, &migrated) {
        cleanup_native_secret(migrated.storage, &migrated.key_id);
        return Err(error);
    }
    Ok(migrated)
}

fn cleanup_native_secret(storage: SecretStorage, key_id: &str) {
    if storage == SecretStorage::NativeKeyring {
        let _ = secret_store::delete_device_identity(key_id);
    }
}

fn json_version(bytes: &[u8], label: &str) -> anyhow::Result<u8> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).with_context(|| format!("decode {label}"))?;
    value["version"]
        .as_u64()
        .and_then(|value| u8::try_from(value).ok())
        .ok_or_else(|| anyhow!("{label} has no valid version"))
}

fn enrollment_url(gateway_ws_url: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(gateway_ws_url).context("parse gateway URL")?;
    let scheme = match url.scheme() {
        "ws" => "http",
        "wss" => "https",
        other => return Err(anyhow!("unsupported gateway URL scheme {other}")),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow!("cannot set gateway HTTP scheme"))?;
    url.set_path("/enroll");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn decode_secret(encoded: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .context("decode identity secret")?;
    bytes
        .try_into()
        .map_err(|_| anyhow!("identity secret has invalid length"))
}

fn write_secret_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("identity path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("identity"),
        Uuid::new_v4()
    ));
    let bytes = serde_json::to_vec_pretty(value)?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    #[cfg(not(unix))]
    fs::write(&temp, &bytes)?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&temp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn file_fallback_identity_round_trip() {
        let dir = tempdir().unwrap();
        ensure_state_dir(dir.path()).unwrap();
        let mut secret = [0_u8; 32];
        getrandom::fill(&mut secret).unwrap();
        let file = IdentityFileV2 {
            version: IDENTITY_VERSION,
            device_id: "device".into(),
            device_name: "Laptop".into(),
            key_id: "test-key".into(),
            storage: SecretStorage::FileFallback,
            secret_key: Some(URL_SAFE_NO_PAD.encode(secret)),
        };
        write_secret_json(&dir.path().join("identity.json"), &file).unwrap();
        let loaded = load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.device_id, "device");
        assert_eq!(
            loaded.public_key_bytes(),
            SigningKey::from_bytes(&secret).verifying_key().to_bytes()
        );
    }

    #[test]
    fn v2_native_metadata_never_serializes_secret_field() {
        let file = IdentityFileV2 {
            version: IDENTITY_VERSION,
            device_id: "device".into(),
            device_name: "Laptop".into(),
            key_id: "key-id".into(),
            storage: SecretStorage::NativeKeyring,
            secret_key: None,
        };
        let json = serde_json::to_string(&file).unwrap();
        assert!(!json.contains("secret_key"));
        assert!(json.contains("native_keyring"));
    }
}
