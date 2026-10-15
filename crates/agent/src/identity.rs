use anyhow::{Context, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use infra_protocol::{DeviceHello, signed_auth_message};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub device_name: String,
    signing_key: SigningKey,
}

#[derive(Debug, Serialize, Deserialize)]
struct IdentityFile {
    version: u8,
    device_id: String,
    device_name: String,
    secret_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingIdentity {
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
    let stored: IdentityFile = serde_json::from_slice(&bytes).context("decode identity.json")?;
    if stored.version != 1 {
        return Err(anyhow!(
            "unsupported identity file version {}",
            stored.version
        ));
    }
    let secret = decode_secret(&stored.secret_key)?;
    Ok(Some(DeviceIdentity {
        device_id: stored.device_id,
        device_name: stored.device_name,
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
    let pending = match fs::read(&pending_path) {
        Ok(bytes) => serde_json::from_slice::<PendingIdentity>(&bytes)
            .context("decode pending enrollment identity")?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let mut secret = [0_u8; 32];
            getrandom::fill(&mut secret).map_err(|_| anyhow!("OS random source unavailable"))?;
            let pending = PendingIdentity {
                version: 1,
                request_id: Uuid::new_v4().to_string(),
                device_name: requested_name.to_string(),
                secret_key: URL_SAFE_NO_PAD.encode(secret),
            };
            write_secret_json(&pending_path, &pending)?;
            pending
        }
        Err(err) => return Err(err.into()),
    };
    if pending.version != 1 {
        return Err(anyhow!(
            "unsupported pending identity version {}",
            pending.version
        ));
    }
    let secret = decode_secret(&pending.secret_key)?;
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
    let stored = IdentityFile {
        version: 1,
        device_id: enrolled.device_id.clone(),
        device_name: enrolled.device_name.clone(),
        secret_key: pending.secret_key,
    };
    write_secret_json(&state_dir.join("identity.json"), &stored)?;
    let _ = fs::remove_file(pending_path);
    Ok(Some(DeviceIdentity {
        device_id: enrolled.device_id,
        device_name: enrolled.device_name,
        signing_key,
    }))
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
    fs::rename(&temp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn secret_file_round_trip() {
        let dir = tempdir().unwrap();
        ensure_state_dir(dir.path()).unwrap();
        let mut secret = [0_u8; 32];
        getrandom::fill(&mut secret).unwrap();
        let file = IdentityFile {
            version: 1,
            device_id: "device".into(),
            device_name: "Laptop".into(),
            secret_key: URL_SAFE_NO_PAD.encode(secret),
        };
        write_secret_json(&dir.path().join("identity.json"), &file).unwrap();
        let loaded = load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.device_id, "device");
        assert_eq!(
            loaded.public_key_bytes(),
            SigningKey::from_bytes(&secret).verifying_key().to_bytes()
        );
    }
}
