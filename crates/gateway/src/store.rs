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
    #[error("pairing request does not exist")]
    PairInvalid,
    #[error("pairing approval code does not match")]
    PairMismatch,
    #[error("pairing request has expired")]
    PairExpired,
    #[error("pairing request conflicts with a different identity")]
    PairConflict,
    #[error("too many pending pairing requests")]
    PairCapacity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device_id: String,
    pub device_name: String,
    pub display_name: Option<String>,
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
    pub last_disconnect_reason: Option<String>,
    pub last_disconnect_at_unix_ms: Option<u64>,
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

#[derive(Debug, Clone, Serialize)]
pub struct AuditRecord {
    pub id: i64,
    pub at_unix_ms: u64,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub action: String,
    pub outcome: String,
    pub request_id: Option<String>,
}

/// Lifecycle of a durable cross-device copy job.
///
/// `queued` and `paused` both mean "not currently being driven"; the difference
/// is intent. A queued job has never started, a paused job stopped because an
/// endpoint went away or a chunk outcome was uncertain. Both are resumable, and
/// the driver re-reads the destination's confirmed offset before every slice, so
/// a restart never trusts this row over the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CopyJobState {
    Queued,
    Running,
    Paused,
    Failed,
    Completed,
    Canceled,
}

impl CopyJobState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Failed => "failed",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "paused" => Some(Self::Paused),
            "failed" => Some(Self::Failed),
            "completed" => Some(Self::Completed),
            "canceled" => Some(Self::Canceled),
            _ => None,
        }
    }

    /// Terminal rows are never driven again and are the only rows retention may
    /// delete.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Completed | Self::Canceled)
    }

    /// States the supervisor rebuilds its task list from after a restart.
    pub fn is_resumable(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Paused)
    }
}

/// One durable copy job. `bytes_done` is the last offset the destination
/// confirmed; `chunk_offset` is the offset the driver will request next. They
/// differ only while a chunk is in flight, which is exactly the window a crash
/// can land in, so the driver treats the destination's own status as the truth
/// and this row as the record of what was already confirmed.
#[derive(Debug, Clone, Serialize)]
pub struct CopyJob {
    pub id: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub source_device: String,
    pub dest_device: String,
    pub source_path: String,
    pub dest_path: String,
    pub state: CopyJobState,
    pub bytes_done: u64,
    pub total_bytes: u64,
    pub chunk_offset: u64,
    /// Whole-file digest the source reported when the download began.
    pub sha256: String,
    /// Digest of the bytes this job relayed since `sha256_progress_offset`.
    /// A job that never resumed starts at 0, so a completed job can prove the
    /// relayed bytes match the source digest without trusting the destination.
    pub sha256_progress: String,
    pub sha256_progress_offset: u64,
    pub source_transfer_id: Option<String>,
    pub dest_transfer_id: Option<String>,
    pub last_error: Option<String>,
}

/// The fields a caller supplies when queueing a copy. Everything else starts at
/// its zero value and is filled in by the driver.
#[derive(Debug, Clone, Copy)]
pub struct NewCopyJob<'a> {
    pub source_device: &'a str,
    pub dest_device: &'a str,
    pub source_path: &'a str,
    pub dest_path: &'a str,
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}

/// Per-table retention windows for the cleanup pass.
///
/// The sweep is deliberately table-agnostic: each table gets its own window and
/// its own counter in [`RetentionSweep`], so the durable copy queue joins the
/// same pass without touching the audit policy or adding a second timer. Only
/// the audit window is operator-configurable; the copy windows are fixed so a
/// typo in one environment variable cannot silently shorten them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub audit_retention_days: u64,
    pub copy_completed_retention_days: u64,
    pub copy_failed_retention_days: u64,
}

impl RetentionPolicy {
    pub const DEFAULT_AUDIT_RETENTION_DAYS: u64 = 180;
    /// A finished copy is history once the owner has seen the digest.
    pub const DEFAULT_COPY_COMPLETED_RETENTION_DAYS: u64 = 7;
    /// A failed or canceled copy keeps its error long enough to be diagnosed.
    pub const DEFAULT_COPY_FAILED_RETENTION_DAYS: u64 = 30;

    /// Reads `INFRA_AUDIT_RETENTION_DAYS`. Unset, unparsable and zero values all
    /// fall back to the default: a typo must not silently delete the audit trail.
    pub fn from_env() -> Self {
        let audit_retention_days = std::env::var("INFRA_AUDIT_RETENTION_DAYS")
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|days| *days > 0)
            .unwrap_or(Self::DEFAULT_AUDIT_RETENTION_DAYS);
        Self {
            audit_retention_days,
            ..Self::default()
        }
    }
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            audit_retention_days: Self::DEFAULT_AUDIT_RETENTION_DAYS,
            copy_completed_retention_days: Self::DEFAULT_COPY_COMPLETED_RETENTION_DAYS,
            copy_failed_retention_days: Self::DEFAULT_COPY_FAILED_RETENTION_DAYS,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetentionSweep {
    pub audit_events_deleted: usize,
    pub copy_jobs_deleted: usize,
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

    pub(super) fn connection(&self) -> Result<Connection, StoreError> {
        let conn = Connection::open(&self.path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(3))?;
        Ok(conn)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        let conn = self.connection()?;
        let mut version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
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
            version = 1;
        }
        if version < 2 {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS audit_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    at_ms INTEGER NOT NULL,
                    device_id TEXT,
                    action TEXT NOT NULL,
                    outcome TEXT NOT NULL,
                    request_id TEXT,
                    FOREIGN KEY(device_id) REFERENCES devices(device_id)
                );
                CREATE INDEX IF NOT EXISTS audit_events_at_idx ON audit_events(at_ms DESC);
                CREATE INDEX IF NOT EXISTS audit_events_device_idx ON audit_events(device_id, at_ms DESC);
                PRAGMA user_version=2;
                "#,
            )?;
        }
        if version < 3 {
            conn.execute_batch(
                "ALTER TABLE devices ADD COLUMN last_disconnect_reason TEXT;
                 ALTER TABLE devices ADD COLUMN last_disconnect_at_ms INTEGER;
                 PRAGMA user_version=3;",
            )?;
        }
        if version < 4 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS pair_requests (
                    request_id TEXT PRIMARY KEY,
                    code TEXT NOT NULL,
                    public_key BLOB NOT NULL,
                    device_name TEXT NOT NULL,
                    platform TEXT NOT NULL,
                    arch TEXT NOT NULL,
                    agent_version TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    expires_at_ms INTEGER NOT NULL,
                    approved_at_ms INTEGER,
                    device_id TEXT,
                    FOREIGN KEY(device_id) REFERENCES devices(device_id)
                 );
                 CREATE INDEX IF NOT EXISTS pair_requests_expires_idx ON pair_requests(expires_at_ms);
                 PRAGMA user_version=4;",
            )?;
        }
        if version < 5 {
            conn.execute_batch(
                "ALTER TABLE devices ADD COLUMN display_name TEXT;
                 PRAGMA user_version=5;",
            )?;
        }
        if version < 6 {
            // No foreign key to devices on purpose: a job outlives a revoked or
            // re-enrolled device, and the queue must not block device lifecycle.
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS copy_jobs (
                    id TEXT PRIMARY KEY,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    source_device TEXT NOT NULL,
                    dest_device TEXT NOT NULL,
                    source_path TEXT NOT NULL,
                    dest_path TEXT NOT NULL,
                    state TEXT NOT NULL,
                    bytes_done INTEGER NOT NULL DEFAULT 0,
                    total_bytes INTEGER NOT NULL DEFAULT 0,
                    chunk_offset INTEGER NOT NULL DEFAULT 0,
                    sha256 TEXT NOT NULL DEFAULT '',
                    sha256_progress TEXT NOT NULL DEFAULT '',
                    sha256_progress_offset INTEGER NOT NULL DEFAULT 0,
                    source_transfer_id TEXT,
                    dest_transfer_id TEXT,
                    last_error TEXT
                );
                CREATE INDEX IF NOT EXISTS copy_jobs_state_idx
                    ON copy_jobs(state, created_at_ms);
                PRAGMA user_version=6;
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
               revoked, reconcile_json, last_disconnect_reason, last_disconnect_at_ms, display_name FROM devices WHERE device_id=?1"#,
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
               revoked, reconcile_json, last_disconnect_reason, last_disconnect_at_ms, display_name FROM devices ORDER BY COALESCE(display_name, device_name) COLLATE NOCASE"#,
        )?;
        let rows = stmt.query_map([], decode_device)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn set_device_alias(
        &self,
        device_id: &str,
        name: Option<&str>,
    ) -> Result<bool, StoreError> {
        let name = name.map(str::trim).filter(|value| !value.is_empty());
        if name.is_some_and(|value| value.len() > 120 || value.contains(['\n', '\r', '\0'])) {
            return Err(StoreError::PairInvalid);
        }
        let conn = self.connection()?;
        Ok(conn.execute(
            "UPDATE devices SET display_name=?2 WHERE device_id=?1 AND revoked=0",
            params![device_id, name],
        )? > 0)
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

    pub fn record_disconnect(&self, device_id: &str, reason: &str) -> Result<(), StoreError> {
        let conn = self.connection()?;
        // Reason is a fixed backend classification, not raw OS/network error content.
        conn.execute(
            "UPDATE devices SET last_seen_ms=?1, last_disconnect_at_ms=?1, last_disconnect_reason=?3 WHERE device_id=?2",
            params![now_ms() as i64, device_id, reason],
        )?;
        Ok(())
    }

    /// Records a `device.recovered` audit event when the device had a disconnect
    /// recorded within the last 24 hours, and returns the elapsed seconds.
    ///
    /// The disconnect timestamp is deliberately left in place: the dashboard's
    /// drop note keeps showing the last drop after the device comes back. A
    /// gateway restart therefore re-reports the outage it interrupted, which is
    /// the measurement the soak program wants rather than a duplicate.
    pub fn record_recovery(&self, device_id: &str) -> Result<Option<u64>, StoreError> {
        const RECOVERY_WINDOW_MS: u64 = 24 * 60 * 60 * 1000;
        let conn = self.connection()?;
        let last_disconnect = conn
            .query_row(
                "SELECT last_disconnect_at_ms FROM devices WHERE device_id=?1",
                params![device_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
            .map(|value| value.max(0) as u64);
        let Some(last_disconnect) = last_disconnect else {
            return Ok(None);
        };
        let now = now_ms();
        let elapsed = now.saturating_sub(last_disconnect);
        if elapsed > RECOVERY_WINDOW_MS {
            return Ok(None);
        }
        let seconds = elapsed / 1000;
        conn.execute(
            "INSERT INTO audit_events(at_ms, device_id, action, outcome, request_id) VALUES (?1, ?2, 'device.recovered', ?3, NULL)",
            params![now as i64, device_id, format!("{seconds}s")],
        )?;
        Ok(Some(seconds))
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

    /// Liveness-only touch for a reconcile that carried no new content.
    pub fn touch_device_seen(&self, device_id: &str) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE devices SET last_seen_ms=?1 WHERE device_id=?2",
            params![now_ms() as i64, device_id],
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

    pub fn record_activity(
        &self,
        device_id: Option<&str>,
        action: &str,
        outcome: &str,
        request_id: Option<&str>,
    ) -> Result<(), StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO audit_events(at_ms, device_id, action, outcome, request_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![now_ms() as i64, device_id, action, outcome, request_id],
        )?;
        Ok(())
    }

    pub fn list_activity(&self, limit: usize) -> Result<Vec<AuditRecord>, StoreError> {
        let conn = self.connection()?;
        let limit = limit.clamp(1, 200) as i64;
        let mut stmt = conn.prepare(
            r#"SELECT a.id, a.at_ms, a.device_id, d.device_name, a.action, a.outcome, a.request_id
               FROM audit_events a
               LEFT JOIN devices d ON d.device_id=a.device_id
               ORDER BY a.at_ms DESC, a.id DESC
               LIMIT ?1"#,
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(AuditRecord {
                id: row.get(0)?,
                at_unix_ms: row.get::<_, i64>(1)?.max(0) as u64,
                device_id: row.get(2)?,
                device_name: row.get(3)?,
                action: row.get(4)?,
                outcome: row.get(5)?,
                request_id: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Deletes rows past their retention window. The audit delete walks the
    /// existing `audit_events_at_idx` index, so it stays cheap on a box that has
    /// been running for months.
    pub fn sweep_retention(&self, policy: RetentionPolicy) -> Result<RetentionSweep, StoreError> {
        const DAY_MS: u64 = 24 * 60 * 60 * 1000;
        let conn = self.connection()?;
        let now = now_ms();
        let audit_cutoff = now.saturating_sub(policy.audit_retention_days.saturating_mul(DAY_MS));
        let audit_events_deleted = conn.execute(
            "DELETE FROM audit_events WHERE at_ms < ?1",
            params![audit_cutoff as i64],
        )?;
        // Copy rows age from their last transition, not from creation: a job
        // created ten days ago that finished yesterday keeps its full window.
        let completed_cutoff =
            now.saturating_sub(policy.copy_completed_retention_days.saturating_mul(DAY_MS));
        let failed_cutoff =
            now.saturating_sub(policy.copy_failed_retention_days.saturating_mul(DAY_MS));
        let copy_jobs_deleted = conn.execute(
            "DELETE FROM copy_jobs WHERE state='completed' AND updated_at_ms < ?1",
            params![completed_cutoff as i64],
        )? + conn.execute(
            "DELETE FROM copy_jobs WHERE state IN ('failed','canceled') AND updated_at_ms < ?1",
            params![failed_cutoff as i64],
        )?;
        Ok(RetentionSweep {
            audit_events_deleted,
            copy_jobs_deleted,
        })
    }

    pub fn create_copy_job(&self, job: NewCopyJob<'_>) -> Result<CopyJob, StoreError> {
        let id = Uuid::new_v4().to_string();
        let now = now_ms();
        let conn = self.connection()?;
        conn.execute(
            r#"INSERT INTO copy_jobs(
                id, created_at_ms, updated_at_ms, source_device, dest_device,
                source_path, dest_path, state, bytes_done, total_bytes, chunk_offset,
                sha256, sha256_progress, sha256_progress_offset
            ) VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, 'queued', 0, 0, 0, '', '', 0)"#,
            params![
                id,
                now as i64,
                job.source_device,
                job.dest_device,
                job.source_path,
                job.dest_path
            ],
        )?;
        Ok(CopyJob {
            id,
            created_at_ms: now,
            updated_at_ms: now,
            source_device: job.source_device.to_owned(),
            dest_device: job.dest_device.to_owned(),
            source_path: job.source_path.to_owned(),
            dest_path: job.dest_path.to_owned(),
            state: CopyJobState::Queued,
            bytes_done: 0,
            total_bytes: 0,
            chunk_offset: 0,
            sha256: String::new(),
            sha256_progress: String::new(),
            sha256_progress_offset: 0,
            source_transfer_id: None,
            dest_transfer_id: None,
            last_error: None,
        })
    }

    pub fn get_copy_job(&self, id: &str) -> Result<Option<CopyJob>, StoreError> {
        let conn = self.connection()?;
        conn.query_row(
            &format!("SELECT {COPY_JOB_COLUMNS} FROM copy_jobs WHERE id=?1"),
            params![id],
            decode_copy_job,
        )
        .optional()
        .map_err(StoreError::from)
    }

    /// Newest first, bounded so a status call cannot return an unbounded list.
    pub fn list_copy_jobs(&self, limit: usize) -> Result<Vec<CopyJob>, StoreError> {
        let conn = self.connection()?;
        let limit = limit.clamp(1, 100) as i64;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COPY_JOB_COLUMNS} FROM copy_jobs ORDER BY created_at_ms DESC, id DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![limit], decode_copy_job)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Every row the supervisor must drive after a restart. A row left in
    /// `running` by a killed gateway has no task behind it, which is why it is
    /// rebuilt here rather than trusted.
    pub fn list_resumable_copy_jobs(&self) -> Result<Vec<CopyJob>, StoreError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COPY_JOB_COLUMNS} FROM copy_jobs
             WHERE state IN ('queued','running','paused')
             ORDER BY created_at_ms, id"
        ))?;
        let rows = stmt.query_map([], decode_copy_job)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Write-ahead transition into `running`. Returns false when the row is
    /// already terminal, so a driver that lost a race with a cancel stops
    /// instead of resurrecting the job.
    pub fn mark_copy_job_running(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.connection()?;
        Ok(conn.execute(
            "UPDATE copy_jobs SET state='running', updated_at_ms=?2
             WHERE id=?1 AND state IN ('queued','running','paused')",
            params![id, now_ms() as i64],
        )? > 0)
    }

    /// Records the two transfer IDs and the source's declared size/digest before
    /// the first chunk is relayed, so a restart resumes the same transfers.
    pub fn record_copy_job_transfers(
        &self,
        id: &str,
        source_transfer_id: &str,
        dest_transfer_id: &str,
        total_bytes: u64,
        sha256: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.connection()?;
        Ok(conn.execute(
            "UPDATE copy_jobs SET source_transfer_id=?2, dest_transfer_id=?3,
                total_bytes=?4, sha256=?5, updated_at_ms=?6
             WHERE id=?1 AND state NOT IN ('failed','completed','canceled')",
            params![
                id,
                source_transfer_id,
                dest_transfer_id,
                total_bytes as i64,
                sha256,
                now_ms() as i64
            ],
        )? > 0)
    }

    /// Records the resume offset and the running digest. Called before a chunk
    /// (with the offset about to be requested) and after it (with the offset the
    /// destination confirmed), so the row never claims more than the destination
    /// has.
    pub fn record_copy_job_progress(
        &self,
        id: &str,
        bytes_done: u64,
        chunk_offset: u64,
        sha256_progress: &str,
        sha256_progress_offset: u64,
    ) -> Result<bool, StoreError> {
        let conn = self.connection()?;
        Ok(conn.execute(
            "UPDATE copy_jobs SET bytes_done=?2, chunk_offset=?3, sha256_progress=?4,
                sha256_progress_offset=?5, updated_at_ms=?6
             WHERE id=?1 AND state NOT IN ('failed','completed','canceled')",
            params![
                id,
                bytes_done as i64,
                chunk_offset as i64,
                sha256_progress,
                sha256_progress_offset as i64,
                now_ms() as i64
            ],
        )? > 0)
    }

    /// Unconditional terminal transition. `last_error` is cleared on success so a
    /// completed job does not keep a stale pause reason.
    pub fn set_copy_job_state(
        &self,
        id: &str,
        state: CopyJobState,
        last_error: Option<&str>,
    ) -> Result<bool, StoreError> {
        let conn = self.connection()?;
        Ok(conn.execute(
            "UPDATE copy_jobs SET state=?2, last_error=?3, updated_at_ms=?4 WHERE id=?1",
            params![id, state.as_str(), last_error, now_ms() as i64],
        )? > 0)
    }

    /// Marks a job canceled unless it already finished. Returns the row as it
    /// stands afterwards, so the caller can report the real state.
    pub fn cancel_copy_job(&self, id: &str) -> Result<Option<CopyJob>, StoreError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE copy_jobs SET state='canceled', last_error='canceled by the owner',
                updated_at_ms=?2
             WHERE id=?1 AND state NOT IN ('failed','completed','canceled')",
            params![id, now_ms() as i64],
        )?;
        drop(conn);
        self.get_copy_job(id)
    }
}

const COPY_JOB_COLUMNS: &str = "id, created_at_ms, updated_at_ms, source_device, dest_device,
    source_path, dest_path, state, bytes_done, total_bytes, chunk_offset, sha256,
    sha256_progress, sha256_progress_offset, source_transfer_id, dest_transfer_id, last_error";

fn decode_copy_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<CopyJob> {
    let state: String = row.get(7)?;
    Ok(CopyJob {
        id: row.get(0)?,
        created_at_ms: row.get::<_, i64>(1)?.max(0) as u64,
        updated_at_ms: row.get::<_, i64>(2)?.max(0) as u64,
        source_device: row.get(3)?,
        dest_device: row.get(4)?,
        source_path: row.get(5)?,
        dest_path: row.get(6)?,
        // An unknown state is a corrupt row, not a reason to panic: report it as
        // failed so the supervisor stops driving it and an operator sees why.
        state: CopyJobState::parse(&state).unwrap_or(CopyJobState::Failed),
        bytes_done: row.get::<_, i64>(8)?.max(0) as u64,
        total_bytes: row.get::<_, i64>(9)?.max(0) as u64,
        chunk_offset: row.get::<_, i64>(10)?.max(0) as u64,
        sha256: row.get(11)?,
        sha256_progress: row.get(12)?,
        sha256_progress_offset: row.get::<_, i64>(13)?.max(0) as u64,
        source_transfer_id: row.get(14)?,
        dest_transfer_id: row.get(15)?,
        last_error: row.get(16)?,
    })
}

fn decode_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRecord> {
    let capabilities_json: String = row.get(7)?;
    let metadata_json: String = row.get(8)?;
    let reconcile_json: Option<String> = row.get(12)?;
    Ok(DeviceRecord {
        device_id: row.get(0)?,
        device_name: row.get(1)?,
        display_name: row.get(15)?,
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
        last_disconnect_reason: row.get(13)?,
        last_disconnect_at_unix_ms: row
            .get::<_, Option<i64>>(14)?
            .map(|value| value.max(0) as u64),
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
    fn display_alias_survives_database_reopen_and_does_not_change_signed_name() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("db");
        let store = Store::open(&db).unwrap();
        let enroll = store.create_enrollment(60_000).unwrap();
        let device = store
            .consume_enrollment(
                &enroll.code,
                "one",
                "DESKTOP-123",
                &[22u8; 32],
                "windows",
                "x86_64",
                "test",
            )
            .unwrap();
        assert!(
            store
                .set_device_alias(&device.device_id, Some("Work PC"))
                .unwrap()
        );
        drop(store);
        let reopened = Store::open(&db).unwrap();
        let record = reopened.get_device(&device.device_id).unwrap().unwrap();
        assert_eq!(record.device_name, "DESKTOP-123");
        assert_eq!(record.display_name.as_deref(), Some("Work PC"));
        assert!(reopened.set_device_alias(&device.device_id, None).unwrap());
        assert_eq!(
            reopened
                .get_device(&device.device_id)
                .unwrap()
                .unwrap()
                .display_name,
            None
        );
    }

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
    fn audit_events_are_bounded_and_join_device_name() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("infra.db")).unwrap();
        let enrollment = store.create_enrollment(60_000).unwrap();
        let enrolled = store
            .consume_enrollment(
                &enrollment.code,
                "audit-request",
                "Audit Laptop",
                &[9_u8; 32],
                "linux",
                "x86_64",
                "test",
            )
            .unwrap();
        store
            .record_activity(Some(&enrolled.device_id), "exec", "ok", Some("req-1"))
            .unwrap();
        let activity = store.list_activity(10).unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].device_name.as_deref(), Some("Audit Laptop"));
        assert_eq!(activity[0].action, "exec");
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
        store
            .record_disconnect(&result.device_id, "heartbeat_timeout")
            .unwrap();
        drop(store);
        let reopened = Store::open(&path).unwrap();
        let device = reopened.get_device(&result.device_id).unwrap().unwrap();
        assert_eq!(device.device_name, "Oracle");
        assert!(!device.revoked);
        assert_eq!(
            device.last_disconnect_reason.as_deref(),
            Some("heartbeat_timeout")
        );
        assert!(device.last_disconnect_at_unix_ms.is_some());
    }

    #[test]
    fn recovery_events_carry_the_elapsed_seconds_and_expire_after_a_day() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("infra.db")).unwrap();
        let enrollment = store.create_enrollment(60_000).unwrap();
        let device = store
            .consume_enrollment(
                &enrollment.code,
                "recovery-request",
                "Recovering Laptop",
                &[3_u8; 32],
                "linux",
                "aarch64",
                "test",
            )
            .unwrap();
        // A device that has never dropped has nothing to recover from.
        assert_eq!(store.record_recovery(&device.device_id).unwrap(), None);
        assert!(store.list_activity(10).unwrap().is_empty());

        store
            .record_disconnect(&device.device_id, "heartbeat_timeout")
            .unwrap();
        assert_eq!(store.record_recovery(&device.device_id).unwrap(), Some(0));
        let activity = store.list_activity(10).unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].action, "device.recovered");
        assert_eq!(activity[0].outcome, "0s");
        assert_eq!(
            activity[0].device_name.as_deref(),
            Some("Recovering Laptop")
        );

        // A disconnect older than the 24h window is history, not a recovery.
        let conn = store.connection().unwrap();
        conn.execute(
            "UPDATE devices SET last_disconnect_at_ms=?1 WHERE device_id=?2",
            params![(now_ms() - 25 * 60 * 60 * 1000) as i64, device.device_id],
        )
        .unwrap();
        drop(conn);
        assert_eq!(store.record_recovery(&device.device_id).unwrap(), None);
        assert_eq!(store.list_activity(10).unwrap().len(), 1);
    }

    #[test]
    fn retention_sweep_deletes_only_rows_older_than_the_window() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("infra.db")).unwrap();
        let enrollment = store.create_enrollment(60_000).unwrap();
        let device = store
            .consume_enrollment(
                &enrollment.code,
                "retention-request",
                "Retention Laptop",
                &[4_u8; 32],
                "linux",
                "aarch64",
                "test",
            )
            .unwrap();
        store
            .record_activity(Some(&device.device_id), "exec", "ok", None)
            .unwrap();
        // The public API always writes "now", so seed the stale rows directly.
        let conn = store.connection().unwrap();
        for days_ago in [181_u64, 400] {
            conn.execute(
                "INSERT INTO audit_events(at_ms, device_id, action, outcome, request_id) VALUES (?1, ?2, 'exec', 'ok', NULL)",
                params![
                    (now_ms() - days_ago * 24 * 60 * 60 * 1000) as i64,
                    device.device_id
                ],
            )
            .unwrap();
        }
        drop(conn);
        assert_eq!(store.list_activity(10).unwrap().len(), 3);

        let sweep = store.sweep_retention(RetentionPolicy::default()).unwrap();
        assert_eq!(sweep.audit_events_deleted, 2);
        let remaining = store.list_activity(10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].action, "exec");
        assert_eq!(
            remaining[0].device_name.as_deref(),
            Some("Retention Laptop")
        );

        // A second sweep inside the window is a no-op, not a second deletion.
        assert_eq!(
            store
                .sweep_retention(RetentionPolicy::default())
                .unwrap()
                .audit_events_deleted,
            0
        );
        assert_eq!(store.list_activity(10).unwrap().len(), 1);
    }

    #[test]
    fn the_default_retention_window_is_180_days() {
        assert_eq!(
            RetentionPolicy::default().audit_retention_days,
            RetentionPolicy::DEFAULT_AUDIT_RETENTION_DAYS
        );
        assert_eq!(RetentionPolicy::DEFAULT_AUDIT_RETENTION_DAYS, 180);
        assert_eq!(
            RetentionPolicy::default().copy_completed_retention_days,
            RetentionPolicy::DEFAULT_COPY_COMPLETED_RETENTION_DAYS
        );
        assert_eq!(
            RetentionPolicy::default().copy_failed_retention_days,
            RetentionPolicy::DEFAULT_COPY_FAILED_RETENTION_DAYS
        );
        assert_eq!(RetentionPolicy::DEFAULT_COPY_COMPLETED_RETENTION_DAYS, 7);
        assert_eq!(RetentionPolicy::DEFAULT_COPY_FAILED_RETENTION_DAYS, 30);
    }

    #[test]
    fn copy_job_states_classify_terminal_and_resumable_rows() {
        for (state, text) in [
            (CopyJobState::Queued, "queued"),
            (CopyJobState::Running, "running"),
            (CopyJobState::Paused, "paused"),
            (CopyJobState::Failed, "failed"),
            (CopyJobState::Completed, "completed"),
            (CopyJobState::Canceled, "canceled"),
        ] {
            assert_eq!(state.as_str(), text);
            assert_eq!(CopyJobState::parse(text), Some(state));
        }
        assert_eq!(CopyJobState::parse("unknown"), None);
        assert!(CopyJobState::Completed.is_terminal());
        assert!(CopyJobState::Failed.is_terminal());
        assert!(CopyJobState::Canceled.is_terminal());
        assert!(!CopyJobState::Paused.is_terminal());
        assert!(CopyJobState::Queued.is_resumable());
        assert!(CopyJobState::Running.is_resumable());
        assert!(CopyJobState::Paused.is_resumable());
        assert!(!CopyJobState::Completed.is_resumable());
    }

    #[test]
    fn a_copy_job_survives_reopen_and_records_its_progress() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("infra.db");
        let store = Store::open(&path).unwrap();
        let job = store
            .create_copy_job(NewCopyJob {
                source_device: "source-id",
                dest_device: "dest-id",
                source_path: "/source.bin",
                dest_path: "/destination.bin",
            })
            .unwrap();
        assert_eq!(job.state, CopyJobState::Queued);
        assert_eq!(job.bytes_done, 0);
        assert!(store.mark_copy_job_running(&job.id).unwrap());
        assert!(
            store
                .record_copy_job_transfers(&job.id, "source-transfer", "dest-transfer", 4096, "abc")
                .unwrap()
        );
        assert!(
            store
                .record_copy_job_progress(&job.id, 1024, 1024, "def", 0)
                .unwrap()
        );
        drop(store);

        // A restart rebuilds the task list from the table, so the row must carry
        // everything the driver needs to resume the same two transfers.
        let reopened = Store::open(&path).unwrap();
        let resumable = reopened.list_resumable_copy_jobs().unwrap();
        assert_eq!(resumable.len(), 1);
        let job = &resumable[0];
        assert_eq!(job.state, CopyJobState::Running);
        assert_eq!(job.bytes_done, 1024);
        assert_eq!(job.chunk_offset, 1024);
        assert_eq!(job.total_bytes, 4096);
        assert_eq!(job.sha256, "abc");
        assert_eq!(job.sha256_progress, "def");
        assert_eq!(job.source_transfer_id.as_deref(), Some("source-transfer"));
        assert_eq!(job.dest_transfer_id.as_deref(), Some("dest-transfer"));

        assert!(
            reopened
                .set_copy_job_state(&job.id, CopyJobState::Completed, None)
                .unwrap()
        );
        let done = reopened.get_copy_job(&job.id).unwrap().unwrap();
        assert_eq!(done.state, CopyJobState::Completed);
        assert_eq!(done.last_error, None);
        assert!(reopened.list_resumable_copy_jobs().unwrap().is_empty());
        assert_eq!(reopened.list_copy_jobs(10).unwrap().len(), 1);
    }

    #[test]
    fn a_terminal_copy_job_rejects_further_progress_and_cancel() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("infra.db")).unwrap();
        let job = store
            .create_copy_job(NewCopyJob {
                source_device: "source-id",
                dest_device: "dest-id",
                source_path: "/source.bin",
                dest_path: "/destination.bin",
            })
            .unwrap();
        assert!(store.mark_copy_job_running(&job.id).unwrap());
        let canceled = store.cancel_copy_job(&job.id).unwrap().unwrap();
        assert_eq!(canceled.state, CopyJobState::Canceled);
        assert_eq!(
            canceled.last_error.as_deref(),
            Some("canceled by the owner")
        );

        // A driver that lost the race with the cancel must not resurrect the job.
        assert!(!store.mark_copy_job_running(&job.id).unwrap());
        assert!(
            !store
                .record_copy_job_progress(&job.id, 512, 512, "aa", 0)
                .unwrap()
        );
        assert!(
            !store
                .record_copy_job_transfers(&job.id, "s", "d", 10, "bb")
                .unwrap()
        );
        let after = store.get_copy_job(&job.id).unwrap().unwrap();
        assert_eq!(after.state, CopyJobState::Canceled);
        assert_eq!(after.bytes_done, 0);
        // Canceling twice is idempotent and keeps the original reason.
        assert_eq!(
            store.cancel_copy_job(&job.id).unwrap().unwrap().state,
            CopyJobState::Canceled
        );
        assert!(store.get_copy_job("missing").unwrap().is_none());
    }

    #[test]
    fn copy_job_retention_uses_the_terminal_transition_time() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("infra.db")).unwrap();
        let mut ids = Vec::new();
        for (state, days_ago) in [
            (CopyJobState::Completed, 8_u64),
            (CopyJobState::Completed, 1),
            (CopyJobState::Failed, 31),
            (CopyJobState::Failed, 2),
            (CopyJobState::Canceled, 31),
            (CopyJobState::Paused, 400),
            (CopyJobState::Queued, 400),
            (CopyJobState::Running, 400),
        ] {
            let job = store
                .create_copy_job(NewCopyJob {
                    source_device: "source-id",
                    dest_device: "dest-id",
                    source_path: "/source.bin",
                    dest_path: "/destination.bin",
                })
                .unwrap();
            store
                .set_copy_job_state(&job.id, state, Some("seeded"))
                .unwrap();
            // Age the row directly: the public API always writes "now".
            let conn = store.connection().unwrap();
            conn.execute(
                "UPDATE copy_jobs SET updated_at_ms=?1 WHERE id=?2",
                params![(now_ms() - days_ago * 24 * 60 * 60 * 1000) as i64, job.id],
            )
            .unwrap();
            ids.push((job.id, state));
        }

        let sweep = store.sweep_retention(RetentionPolicy::default()).unwrap();
        assert_eq!(
            sweep.copy_jobs_deleted, 3,
            "one completed, one failed, one canceled"
        );
        assert_eq!(sweep.audit_events_deleted, 0);
        let remaining: Vec<_> = store
            .list_copy_jobs(100)
            .unwrap()
            .into_iter()
            .map(|job| job.state)
            .collect();
        assert_eq!(remaining.len(), 5);
        assert!(remaining.contains(&CopyJobState::Completed));
        assert!(remaining.contains(&CopyJobState::Failed));
        assert!(remaining.contains(&CopyJobState::Paused));
        assert!(remaining.contains(&CopyJobState::Queued));
        assert!(remaining.contains(&CopyJobState::Running));
        // A second sweep inside the window is a no-op.
        assert_eq!(
            store
                .sweep_retention(RetentionPolicy::default())
                .unwrap()
                .copy_jobs_deleted,
            0
        );
        assert_eq!(store.list_copy_jobs(100).unwrap().len(), 5);
    }

    #[test]
    fn migration_reaches_version_six_and_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("infra.db");
        let store = Store::open(&path).unwrap();
        let version: u32 = store
            .connection()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 6, "copy_jobs is migration 6");
        drop(store);
        // Reopening an already-migrated database must not re-run the migration.
        let reopened = Store::open(&path).unwrap();
        let version: u32 = reopened
            .connection()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 6);
        assert!(reopened.list_copy_jobs(10).unwrap().is_empty());
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
