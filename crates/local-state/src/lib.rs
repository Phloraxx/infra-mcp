use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const CONFIG_VERSION: u8 = 1;
pub const RUNTIME_VERSION: u8 = 1;
pub const CONNECTION_PROOF_VERSION: u8 = 1;
pub const MAX_CONNECTION_PROOF_AGE_MS: u64 = 30_000;

/// A local receipt of a fresh nonce/status roundtrip over the signed socket.
/// This contains no credentials and is not an OS/publisher attestation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionProof {
    pub version: u8,
    pub pid: u32,
    pub device_id: String,
    pub gateway_fingerprint: String,
    pub gateway_epoch: Uuid,
    pub agent_instance_id: Uuid,
    pub connection_generation: u64,
    pub probe_started_unix_ms: u64,
    pub verified_unix_ms: u64,
}

pub fn gateway_fingerprint(url: &str) -> String {
    hex::encode(Sha256::digest(url.as_bytes()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalConfig {
    pub version: u8,
    pub gateway_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
}

impl LocalConfig {
    pub fn new(gateway_url: String, device_name: Option<String>) -> anyhow::Result<Self> {
        if !matches!(gateway_url.split(':').next(), Some("ws" | "wss")) {
            return Err(anyhow!("gateway URL must use ws:// or wss://"));
        }
        if gateway_url.trim().is_empty() {
            return Err(anyhow!("gateway URL cannot be empty"));
        }
        if device_name
            .as_deref()
            .is_some_and(|name| name.trim().is_empty())
        {
            return Err(anyhow!("device name cannot be blank"));
        }
        Ok(Self {
            version: CONFIG_VERSION,
            gateway_url,
            device_name,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeStatus {
    pub version: u8,
    pub pid: u32,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_epoch: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_instance_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connected_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_storage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_verified_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub updated_unix_ms: u64,
}

impl RuntimeStatus {
    pub fn new(state: impl Into<String>) -> Self {
        Self {
            version: RUNTIME_VERSION,
            pid: std::process::id(),
            state: state.into(),
            device_id: None,
            device_name: None,
            connection_generation: None,
            gateway_epoch: None,
            agent_instance_id: None,
            last_connected_unix_ms: None,
            credential_storage: None,
            credential_verified_unix_ms: None,
            last_error: None,
            updated_unix_ms: now_ms(),
        }
    }

    pub fn touch(mut self) -> Self {
        self.updated_unix_ms = now_ms();
        self
    }
}

pub fn default_state_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("INFRA_STATE_DIR") {
        return PathBuf::from(path);
    }
    #[cfg(windows)]
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(home).join(".infra-mcp-v1");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".infra-mcp-v1");
    }
    std::env::temp_dir().join("infra-mcp-v1")
}

pub fn ensure_state_dir(path: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("create state directory {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn read_config(state_dir: &Path) -> anyhow::Result<Option<LocalConfig>> {
    read_optional_json(
        &state_dir.join("config.json"),
        CONFIG_VERSION,
        |value: &LocalConfig| value.version,
    )
}

pub fn write_config(state_dir: &Path, config: &LocalConfig) -> anyhow::Result<()> {
    ensure_state_dir(state_dir)?;
    atomic_json(&state_dir.join("config.json"), config)
}

pub fn read_runtime(state_dir: &Path) -> anyhow::Result<Option<RuntimeStatus>> {
    read_optional_json(
        &state_dir.join("runtime.json"),
        RUNTIME_VERSION,
        |value: &RuntimeStatus| value.version,
    )
}

pub fn write_runtime(state_dir: &Path, status: &RuntimeStatus) -> anyhow::Result<()> {
    ensure_state_dir(state_dir)?;
    atomic_json(&state_dir.join("runtime.json"), status)
}

pub fn read_connection_proof(state_dir: &Path) -> anyhow::Result<Option<ConnectionProof>> {
    read_optional_json(
        &state_dir.join("connection-proof.json"),
        CONNECTION_PROOF_VERSION,
        |value: &ConnectionProof| value.version,
    )
}

pub fn write_connection_proof(state_dir: &Path, proof: &ConnectionProof) -> anyhow::Result<()> {
    ensure_state_dir(state_dir)?;
    atomic_json(&state_dir.join("connection-proof.json"), proof)
}

pub fn identity_path(state_dir: &Path) -> PathBuf {
    state_dir.join("identity.json")
}

fn read_optional_json<T, F>(
    path: &Path,
    expected_version: u8,
    version: F,
) -> anyhow::Result<Option<T>>
where
    T: for<'de> Deserialize<'de>,
    F: FnOnce(&T) -> u8,
{
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let value: T =
        serde_json::from_slice(&bytes).with_context(|| format!("decode {}", path.display()))?;
    let actual = version(&value);
    if actual != expected_version {
        return Err(anyhow!(
            "unsupported {} version {actual}; expected {expected_version}",
            path.display()
        ));
    }
    Ok(Some(value))
}

fn atomic_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("infra"),
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

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_receipts_are_versioned_and_optional_for_old_runtime_state() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = serde_json::json!({"version":1,"pid":42,"state":"online","updated_unix_ms":1});
        let runtime: RuntimeStatus = serde_json::from_value(legacy).unwrap();
        assert!(runtime.gateway_epoch.is_none() && runtime.agent_instance_id.is_none());
        let mut proof = ConnectionProof {
            version: CONNECTION_PROOF_VERSION,
            pid: 42,
            device_id: "fixture".into(),
            gateway_fingerprint: gateway_fingerprint("ws://127.0.0.1:12345/agent"),
            gateway_epoch: Uuid::new_v4(),
            agent_instance_id: Uuid::new_v4(),
            connection_generation: 7,
            probe_started_unix_ms: 1,
            verified_unix_ms: 1,
        };
        assert!(read_connection_proof(dir.path()).unwrap().is_none());
        write_connection_proof(dir.path(), &proof).unwrap();
        assert_eq!(
            read_connection_proof(dir.path()).unwrap(),
            Some(proof.clone())
        );
        proof.version += 1;
        write_connection_proof(dir.path(), &proof).unwrap();
        assert!(read_connection_proof(dir.path()).is_err());
    }
    use tempfile::tempdir;

    #[test]
    fn config_and_runtime_round_trip() {
        let dir = tempdir().unwrap();
        let config =
            LocalConfig::new("wss://example.invalid/agent".into(), Some("Laptop".into())).unwrap();
        write_config(dir.path(), &config).unwrap();
        assert_eq!(read_config(dir.path()).unwrap(), Some(config));

        let mut runtime = RuntimeStatus::new("online");
        runtime.device_id = Some("device-1".into());
        runtime.connection_generation = Some(4);
        write_runtime(dir.path(), &runtime).unwrap();
        assert_eq!(read_runtime(dir.path()).unwrap(), Some(runtime));
    }

    #[test]
    fn invalid_gateway_scheme_is_rejected() {
        assert!(LocalConfig::new("https://example.invalid".into(), None).is_err());
    }
}
