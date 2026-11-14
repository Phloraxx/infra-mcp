//! Read-only assertions against the gateway's SQLite store.
//!
//! The harness opens the same database file the gateway owns. WAL mode allows a
//! second process to read while the gateway writes, and a busy timeout absorbs
//! the short write windows.

use infra_protocol::ReconcileSnapshot;
use rusqlite::{Connection, OptionalExtension, params};
use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    pub action: String,
    pub outcome: String,
    pub device_id: Option<String>,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeviceRow {
    pub last_disconnect_reason: Option<String>,
    pub last_disconnect_at_ms: Option<u64>,
    pub reconcile: Option<ReconcileSnapshot>,
    pub revoked: bool,
}

/// One durable copy job row, as the gateway wrote it.
#[derive(Debug, Clone)]
pub struct CopyJobRow {
    pub state: String,
    pub bytes_done: u64,
    pub total_bytes: u64,
    pub chunk_offset: u64,
    pub sha256: String,
    pub sha256_progress: String,
    pub sha256_progress_offset: u64,
    pub source_transfer_id: Option<String>,
    pub dest_transfer_id: Option<String>,
    pub last_error: Option<String>,
}

pub struct StoreProbe {
    path: PathBuf,
}

impl StoreProbe {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn connection(&self) -> Connection {
        let connection = Connection::open(&self.path).expect("open gateway store");
        connection
            .busy_timeout(Duration::from_secs(5))
            .expect("set busy timeout");
        connection
    }

    pub fn audit_rows(&self) -> Vec<AuditRow> {
        let connection = self.connection();
        let mut statement = connection
            .prepare("SELECT action, outcome, device_id, request_id FROM audit_events ORDER BY id")
            .expect("prepare audit query");
        statement
            .query_map([], |row| {
                Ok(AuditRow {
                    action: row.get(0)?,
                    outcome: row.get(1)?,
                    device_id: row.get(2)?,
                    request_id: row.get(3)?,
                })
            })
            .expect("query audit rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("read audit rows")
    }

    pub fn audit_count(&self, action: &str) -> usize {
        self.audit_rows()
            .iter()
            .filter(|row| row.action == action)
            .count()
    }

    /// The durable copy job row. Asserting on the row (not only on the MCP view)
    /// is what proves the queue survived a gateway restart.
    pub fn copy_job(&self, job_id: &str) -> Option<CopyJobRow> {
        let connection = self.connection();
        connection
            .query_row(
                "SELECT state, bytes_done, total_bytes, chunk_offset, sha256,
                        sha256_progress, sha256_progress_offset,
                        source_transfer_id, dest_transfer_id, last_error
                 FROM copy_jobs WHERE id=?1",
                params![job_id],
                |row| {
                    Ok(CopyJobRow {
                        state: row.get(0)?,
                        bytes_done: row.get::<_, i64>(1)?.max(0) as u64,
                        total_bytes: row.get::<_, i64>(2)?.max(0) as u64,
                        chunk_offset: row.get::<_, i64>(3)?.max(0) as u64,
                        sha256: row.get(4)?,
                        sha256_progress: row.get(5)?,
                        sha256_progress_offset: row.get::<_, i64>(6)?.max(0) as u64,
                        source_transfer_id: row.get(7)?,
                        dest_transfer_id: row.get(8)?,
                        last_error: row.get(9)?,
                    })
                },
            )
            .optional()
            .expect("read copy job row")
    }

    pub fn device_row(&self, device_id: &str) -> Option<DeviceRow> {
        let connection = self.connection();
        connection
            .query_row(
                "SELECT last_disconnect_reason, last_disconnect_at_ms, reconcile_json, revoked
                 FROM devices WHERE device_id=?1",
                params![device_id],
                |row| {
                    let reconcile: Option<String> = row.get(2)?;
                    Ok(DeviceRow {
                        last_disconnect_reason: row.get(0)?,
                        last_disconnect_at_ms: row.get::<_, Option<i64>>(1)?.map(|v| v as u64),
                        reconcile: reconcile.and_then(|json| serde_json::from_str(&json).ok()),
                        revoked: row.get::<_, i64>(3)? != 0,
                    })
                },
            )
            .optional()
            .expect("read device row")
    }
}
