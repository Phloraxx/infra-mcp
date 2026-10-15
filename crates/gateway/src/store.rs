use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use infra_protocol::{DeviceHello, ReconcileSnapshot};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("storage error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("random source unavailable")]
    Random,
    #[error("enrollment code is invalid")]
    EnrollmentInvalid,
    #[error("enrollment code has expired")]
    EnrollmentExpired,
    #[error("enrollment code was already consumed")]
    EnrollmentConsumed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device_id: String,
    pub device_name: String,
    pub public_key: Option<Vec<u8>>,
    pub platform: String,
    pub arch: String,
    pub agent_version: String,
    pub boot_id: String,
    pub capabilities: Vec<String>,
    pub metadata: serde_json::Value,
    pub paired_at_unix_ms: u64,
    pub last_seen_unix_ms: u64,
    pub revoked: bool,
    pub reconcile: Option<ReconcileSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnrollmentCreated {
    pub enrollment_id: String,
    pub code: String,
    pub expires_at_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnrollmentResult {
    pub device_id: String,
    pub device_name: String,
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let store = Self { path };
        store.migrate()?;
        Ok(store)
    }

    fn connection(&self) -> Result<Connection, StoreError> {
        let conn = Connection::open(&self.path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(3))?;
        Ok(conn)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        let conn = self.connection()?;
        let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version == 0 {
            conn.execute_batch(
                r#"
                PRAGMA journal_mode=WAL;
                CREATE TABLE IF NOT EXISTS devices (
                    device_id TEXT PRIMARY KEY,
                    device_name TEXT NOT NULL,
                    public_key BLOB,
                    platform TEXT NOT NULL DEFAULT '',
                    arch TEXT NOT NULL DEFAULT '',
                    agent_version TEXT NOT NULL DEFAULT '',
                    boot_id TEXT NOT NULL DEFAULT '',
                    capabilities_json TEXT NOT NULL DEFAULT '[]',
                    metadata_json TEXT NOT NULL DEFAULT '{}',
                    paired_at_ms INTEGER NOT NULL,
                    last_seen_ms INTEGER NOT NULL,
                    revoked INTEGER NOT NULL DEFAULT 0,
                    reconcile_json TEXT
                );
                CREATE TABLE IF NOT EXISTS enrollments (
                    enrollment_id TEXT PRIMARY KEY,
                    code_hash BLOB NOT NULL UNIQUE,
                    created_at_ms INTEGER NOT NULL,
                    expires_at_ms INTEGER NOT NULL,
                    consumed_at_ms INTEGER,
                    request_id TEXT,
                    device_id TEXT,
                    public_key BLOB,
                    FOREIGN KEY(device_id) REFERENCES devices(device_id)
                );
                CREATE INDEX IF NOT EXISTS enrollments_expires_idx
                    ON enrollments(expires_at_ms);
                PRAGMA user_version=1;
                "#,
            )?;
        }
        Ok(())
    }

    pub fn create_enrollment(&self, ttl_ms: u64) -> Result<EnrollmentCreated, StoreError> {
        let mut random = [0_u8; 24];
        getrandom::fill(&mut random).map_err(|_| StoreError::Random)?;
        let code = URL_SAFE_NO_PAD.encode(random);
        let code_hash = code_hash(&code);
        let enrollment_id = Uuid::new_v4().to_string();
        let created = now_ms();
        let expires = created.saturating_add(ttl_ms);
        let conn = self.connection()?;
        conn.execute(
            "DELETE FROM enrollments WHERE expires_at_ms < ?1 AND consumed_at_ms IS NULL",
            params![created as i64],
        )?;
        conn.execute(
            "INSERT INTO enrollments(enrollment_id, code_hash, created_at_ms, expires_at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![enrollment_id, code_hash.as_slice(), created as i64, expires as i64],
        )?;
        Ok(EnrollmentCreated {
            enrollment_id,
            code,
            expires_at_unix_ms: expires,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn consume_enrollment(
        &self,
        code: &str,
        request_id: &str,
        device_name: &str,
        public_key: &[u8],
        platform: &str,
        arch: &str,
        agent_version: &str,
    ) -> Result<EnrollmentResult, StoreError> {
        let hash = code_hash(code);
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = tx
            .query_row(
                "SELECT expires_at_ms, consumed_at_ms, request_id, device_id, public_key FROM enrollments WHERE code_hash=?1",
                params![hash.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<Vec<u8>>>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((expires_at, consumed_at, stored_request, stored_device, stored_key)) = row else {
            return Err(StoreError::EnrollmentInvalid);
        };

        if consumed_at.is_some() {
            if stored_request.as_deref() == Some(request_id)
                && stored_key.as_deref() == Some(public_key)
                && let Some(device_id) = stored_device
            {
                let name = tx.query_row(
                    "SELECT device_name FROM devices WHERE device_id=?1",
                    params![device_id],
                    |row| row.get::<_, String>(0),
                )?;
                tx.commit()?;
                return Ok(EnrollmentResult {
                    device_id,
                    device_name: name,
                });
            }
            return Err(StoreError::EnrollmentConsumed);
        }
        if expires_at < now_ms() as i64 {
            return Err(StoreError::EnrollmentExpired);
        }

        let device_id = Uuid::new_v4().to_string();
        let now = now_ms();
        tx.execute(
            r#"INSERT INTO devices(
                device_id, device_name, public_key, platform, arch, agent_version,
                boot_id, capabilities_json, metadata_json, paired_at_ms, last_seen_ms, revoked
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', '[]', '{}', ?7, ?7, 0)"#,
            params![
                device_id,
                device_name,
                public_key,
                platform,
                arch,
                agent_version,
                now as i64
            ],
        )?;
        tx.execute(
            "UPDATE enrollments SET consumed_at_ms=?1, request_id=?2, device_id=?3, public_key=?4 WHERE code_hash=?5",
            params![now as i64, request_id, device_id, public_key, hash.as_slice()],
        )?;
        tx.commit()?;
        Ok(EnrollmentResult {
            device_id,
            device_name: device_name.to_string(),
        })
    }

    pub fn get_device(&self, device_id: &str) -> Result<Option<DeviceRecord>, StoreError> {
        let conn = self.connection()?;
        conn.query_row(
            r#"SELECT device_id, device_name, public_key, platform, arch, agent_version,
               boot_id, capabilities_json, metadata_json, paired_at_ms, last_seen_ms,
               revoked, reconcile_json FROM devices WHERE device_id=?1"#,
            params![device_id],
            decode_device,
        )
        .optional()
        .map_err(StoreError::from)
    }

    pub fn list_devices(&self) -> Result<Vec<DeviceRecord>, StoreError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            r#"SELECT device_id, device_name, public_key, platform, arch, agent_version,
               boot_id, capabilities_json, metadata_json, paired_at_ms, last_seen_ms,
               revoked, reconcile_json FROM devices ORDER BY device_name COLLATE NOCASE"#,
        )?;
        let rows = stmt.query_map([], decode_device)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn record_connect(
        &self,
        hello: &DeviceHello,
        public_key: Option<&[u8]>,
    ) -> Result<(), StoreError> {
        let conn = self.connection()?;
        let now = now_ms();
        let capabilities = serde_json::to_string(&hello.capabilities)?;
        let metadata = serde_json::to_string(&hello.metadata)?;
        conn.execute(
            r#"INSERT INTO devices(
                device_id, device_name, public_key, platform, arch, agent_version, boot_id,
                capabilities_json, metadata_json, paired_at_ms, last_seen_ms, revoked
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 0)
            ON CONFLICT(device_id) DO UPDATE SET
                device_name=excluded.device_name,
                public_key=COALESCE(devices.public_key, excluded.public_key),
                platform=excluded.platform,
                arch=excluded.arch,
                agent_version=excluded.agent_version,
                boot_id=excluded.boot_id,
                capabilities_json=excluded.capabilities_json,
                metadata_json=excluded.metadata_json,
                last_seen_ms=excluded.last_seen_ms"#,
            params![
                hello.device_id,
                hello.device_name,
                public_key,
                hello.platform,
                hello.arch,
                hello.agent_version,
                hello.boot_id,
                capabilities,
                metadata,
                now as i64,
            ],
        )?;
        Ok(())
    }

    pub fn record_disconnect(&self, device_id: &str) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE devices SET last_seen_ms=?1 WHERE device_id=?2",
            params![now_ms() as i64, device_id],
        )?;
        Ok(())
    }

    pub fn record_reconcile(
        &self,
        device_id: &str,
        snapshot: &ReconcileSnapshot,
    ) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE devices SET reconcile_json=?1, last_seen_ms=?2 WHERE device_id=?3",
            params![serde_json::to_string(snapshot)?, now_ms() as i64, device_id],
        )?;
        Ok(())
    }

    pub fn revoke(&self, device_id: &str) -> Result<bool, StoreError> {
        let conn = self.connection()?;
        let changed = conn.execute(
            "UPDATE devices SET revoked=1 WHERE device_id=?1",
            params![device_id],
        )?;
        Ok(changed > 0)
    }
}

fn decode_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRecord> {
    let capabilities_json: String = row.get(7)?;
    let metadata_json: String = row.get(8)?;
    let reconcile_json: Option<String> = row.get(12)?;
    Ok(DeviceRecord {
        device_id: row.get(0)?,
        device_name: row.get(1)?,
        public_key: row.get(2)?,
        platform: row.get(3)?,
        arch: row.get(4)?,
        agent_version: row.get(5)?,
        boot_id: row.get(6)?,
        capabilities: serde_json::from_str(&capabilities_json).unwrap_or_default(),
        metadata: serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null),
        paired_at_unix_ms: row.get::<_, i64>(9)?.max(0) as u64,
        last_seen_unix_ms: row.get::<_, i64>(10)?.max(0) as u64,
        revoked: row.get::<_, i64>(11)? != 0,
        reconcile: reconcile_json.and_then(|json| serde_json::from_str(&json).ok()),
    })
}

fn code_hash(code: &str) -> [u8; 32] {
    Sha256::digest(code.as_bytes()).into()
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
    use tempfile::tempdir;

    #[test]
    fn enrollment_is_single_use_but_idempotent_for_same_request() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("infra.db")).unwrap();
        let enrollment = store.create_enrollment(60_000).unwrap();
        let key = [7_u8; 32];
        let first = store
            .consume_enrollment(
                &enrollment.code,
                "request-a",
                "Laptop",
                &key,
                "darwin",
                "aarch64",
                "test",
            )
            .unwrap();
        let retry = store
            .consume_enrollment(
                &enrollment.code,
                "request-a",
                "Laptop",
                &key,
                "darwin",
                "aarch64",
                "test",
            )
            .unwrap();
        assert_eq!(first.device_id, retry.device_id);
        let err = store
            .consume_enrollment(
                &enrollment.code,
                "request-b",
                "Other",
                &[8_u8; 32],
                "linux",
                "x86_64",
                "test",
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::EnrollmentConsumed));
    }

    #[test]
    fn persisted_device_survives_store_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("infra.db");
        let store = Store::open(&path).unwrap();
        let enrollment = store.create_enrollment(60_000).unwrap();
        let result = store
            .consume_enrollment(
                &enrollment.code,
                "req",
                "Oracle",
                &[1_u8; 32],
                "linux",
                "aarch64",
                "test",
            )
            .unwrap();
        drop(store);
        let reopened = Store::open(&path).unwrap();
        let device = reopened.get_device(&result.device_id).unwrap().unwrap();
        assert_eq!(device.device_name, "Oracle");
        assert!(!device.revoked);
    }

    #[test]
    fn expired_enrollment_is_rejected() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("infra.db")).unwrap();
        let enrollment = store.create_enrollment(0).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let err = store
            .consume_enrollment(
                &enrollment.code,
                "req",
                "Test",
                &[1_u8; 32],
                "linux",
                "x86_64",
                "test",
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::EnrollmentExpired));
    }
}
