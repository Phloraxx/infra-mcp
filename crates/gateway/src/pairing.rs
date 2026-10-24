//! Durable owner-approved device pairing using the existing SQLite store.
//! A comparison code is NOT an authentication credential: the owner must approve
//! and the enrolled device must still prove Ed25519 possession over /agent.
use crate::store::{Store, StoreError, now_ms};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use uuid::Uuid;

const MAX_PENDING: i64 = 64;
const MAX_RECENT_REQUESTS: i64 = 20;
const PAIR_TTL_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone, Serialize)]
pub struct PairView {
    pub request_id: String,
    pub device_name: String,
    pub platform: String,
    pub arch: String,
    pub agent_version: String,
    pub verification_code: String,
    pub expires_at_unix_ms: u64,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

fn row_pair(row: &rusqlite::Row<'_>) -> rusqlite::Result<PairView> {
    let expires: i64 = row.get(6)?;
    let device_id: Option<String> = row.get(8)?;
    let approved: Option<i64> = row.get(7)?;
    let state = if approved.is_some() {
        "approved"
    } else if expires < now_ms() as i64 {
        "expired"
    } else {
        "pending"
    };
    Ok(PairView {
        request_id: row.get(0)?,
        device_name: row.get(2)?,
        platform: row.get(3)?,
        arch: row.get(4)?,
        agent_version: row.get(5)?,
        verification_code: row.get(1)?,
        expires_at_unix_ms: expires.max(0) as u64,
        state,
        device_id,
    })
}

impl Store {
    pub fn request_pair(
        &self,
        request_id: &str,
        public_key: &[u8; 32],
        device_name: &str,
        platform: &str,
        arch: &str,
        version: &str,
    ) -> Result<PairView, StoreError> {
        if Uuid::parse_str(request_id).is_err() {
            return Err(StoreError::PairInvalid);
        }
        let now = now_ms();
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Never delete approved requests during retries: the client must be able
        // to recover its stable assigned ID after its HTTP response is lost.
        tx.execute(
            "DELETE FROM pair_requests WHERE expires_at_ms < ?1 AND approved_at_ms IS NULL",
            params![now.saturating_sub(PAIR_TTL_MS) as i64],
        )?;
        let existing = tx.query_row(
            "SELECT request_id, code, device_name, platform, arch, agent_version, expires_at_ms, approved_at_ms, device_id, public_key FROM pair_requests WHERE request_id=?1",
            params![request_id],
            |row| Ok((row_pair(row)?, row.get::<_, Vec<u8>>(9)?)),
        ).optional()?;
        if let Some((view, stored_key)) = existing {
            if stored_key != public_key
                || view.device_name != device_name
                || view.platform != platform
                || view.arch != arch
                || view.agent_version != version
            {
                return Err(StoreError::PairConflict);
            }
            if view.state != "expired" {
                tx.commit()?;
                return Ok(view);
            }
            let code = new_code()?;
            tx.execute(
                "UPDATE pair_requests SET code=?2, created_at_ms=?3, expires_at_ms=?4 WHERE request_id=?1 AND approved_at_ms IS NULL",
                params![request_id, code, now as i64, now.saturating_add(PAIR_TTL_MS) as i64],
            )?;
        } else {
            let already_enrolled: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM devices WHERE public_key=?1 LIMIT 1",
                    params![public_key],
                    |row| row.get(0),
                )
                .optional()?;
            if already_enrolled.is_some() {
                return Err(StoreError::PairConflict);
            }
            let pending: i64 = tx.query_row(
                "SELECT COUNT(*) FROM pair_requests WHERE approved_at_ms IS NULL AND expires_at_ms>=?1",
                params![now as i64],
                |row| row.get(0),
            )?;
            if pending >= MAX_PENDING {
                return Err(StoreError::PairCapacity);
            }
            let recent: i64 = tx.query_row(
                "SELECT COUNT(*) FROM pair_requests WHERE created_at_ms>=?1",
                params![now.saturating_sub(60_000) as i64],
                |row| row.get(0),
            )?;
            if recent >= MAX_RECENT_REQUESTS {
                return Err(StoreError::PairCapacity);
            }
            tx.execute(
                "INSERT INTO pair_requests(request_id, code, public_key, device_name, platform, arch, agent_version, created_at_ms, expires_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![request_id, new_code()?, public_key, device_name, platform, arch, version, now as i64, now.saturating_add(PAIR_TTL_MS) as i64],
            )?;
        }
        let result = tx.query_row(
            "SELECT request_id, code, device_name, platform, arch, agent_version, expires_at_ms, approved_at_ms, device_id FROM pair_requests WHERE request_id=?1",
            params![request_id],
            row_pair,
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn pair_status(
        &self,
        request_id: &str,
        public_key: &[u8; 32],
    ) -> Result<PairView, StoreError> {
        let conn = self.connection()?;
        let row = conn.query_row(
            "SELECT request_id, code, device_name, platform, arch, agent_version, expires_at_ms, approved_at_ms, device_id, public_key FROM pair_requests WHERE request_id=?1",
            params![request_id],
            |row| Ok((row_pair(row)?, row.get::<_, Vec<u8>>(9)?)),
        ).optional()?;
        match row {
            Some((view, stored_key)) if stored_key == public_key => Ok(view),
            _ => Err(StoreError::PairInvalid),
        }
    }

    pub fn pending_pairs(&self) -> Result<Vec<PairView>, StoreError> {
        let conn = self.connection()?;
        let mut statement = conn.prepare(
            "SELECT request_id, code, device_name, platform, arch, agent_version, expires_at_ms, approved_at_ms, device_id FROM pair_requests WHERE approved_at_ms IS NULL AND expires_at_ms>=?1 ORDER BY created_at_ms DESC LIMIT 64",
        )?;
        statement
            .query_map(params![now_ms() as i64], row_pair)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn approve_pair(&self, request_id: &str, code: &str) -> Result<PairView, StoreError> {
        let mut conn = self.connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = tx.query_row(
            "SELECT code, public_key, device_name, platform, arch, agent_version, expires_at_ms, approved_at_ms, device_id FROM pair_requests WHERE request_id=?1",
            params![request_id],
            |row| Ok((
                row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                row.get::<_, String>(4)?, row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?, row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?
            )),
        ).optional()?;
        let Some((stored_code, key, name, platform, arch, version, expiry, approved, existing_id)) =
            record
        else {
            return Err(StoreError::PairInvalid);
        };
        if stored_code != code {
            return Err(StoreError::PairMismatch);
        }
        if approved.is_some() {
            // Owner double-clicks are idempotent for an already approved pair.
            if existing_id.is_none() {
                return Err(StoreError::PairConflict);
            }
        } else {
            if expiry < now_ms() as i64 {
                return Err(StoreError::PairExpired);
            }
            let collision: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM devices WHERE public_key=?1 LIMIT 1",
                    params![key],
                    |row| row.get(0),
                )
                .optional()?;
            if collision.is_some() {
                return Err(StoreError::PairConflict);
            }
            let id = Uuid::new_v4().to_string();
            let now = now_ms() as i64;
            tx.execute(
                "INSERT INTO devices(device_id, device_name, public_key, platform, arch, agent_version, boot_id, capabilities_json, metadata_json, paired_at_ms, last_seen_ms, revoked) VALUES (?1,?2,?3,?4,?5,?6,'','[]','{}',?7,?7,0)",
                params![id, name, key, platform, arch, version, now],
            )?;
            tx.execute(
                "UPDATE pair_requests SET approved_at_ms=?2, device_id=?3 WHERE request_id=?1",
                params![request_id, now, id],
            )?;
        }
        let result = tx.query_row(
            "SELECT request_id, code, device_name, platform, arch, agent_version, expires_at_ms, approved_at_ms, device_id FROM pair_requests WHERE request_id=?1",
            params![request_id], row_pair,
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn reject_pair(&self, request_id: &str) -> Result<bool, StoreError> {
        let conn = self.connection()?;
        Ok(conn.execute(
            "DELETE FROM pair_requests WHERE request_id=?1 AND approved_at_ms IS NULL",
            params![request_id],
        )? > 0)
    }
}

fn new_code() -> Result<String, StoreError> {
    let mut random = [0u8; 4];
    getrandom::fill(&mut random).map_err(|_| StoreError::Random)?;
    Ok(format!("{:06}", u32::from_le_bytes(random) % 1_000_000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn request_resume_approve_and_key_binding() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("state.sqlite")).unwrap();
        let id = Uuid::new_v4().to_string();
        let key = [42u8; 32];
        let initial = store
            .request_pair(&id, &key, "My PC", "windows", "x86_64", "v1")
            .unwrap();
        assert_eq!(initial.state, "pending");
        assert_eq!(initial.verification_code.len(), 6);
        assert_eq!(store.pending_pairs().unwrap().len(), 1);
        assert_eq!(
            initial.verification_code,
            store
                .request_pair(&id, &key, "My PC", "windows", "x86_64", "v1")
                .unwrap()
                .verification_code
        );
        assert!(matches!(
            store.request_pair(&id, &[1u8; 32], "My PC", "windows", "x86_64", "v1"),
            Err(StoreError::PairConflict)
        ));
        assert!(matches!(
            store.approve_pair(&id, "000000-if-not-code"),
            Err(StoreError::PairMismatch)
        ));
        assert!(matches!(
            store.pair_status(&id, &[1u8; 32]),
            Err(StoreError::PairInvalid)
        ));
        let approved = store.approve_pair(&id, &initial.verification_code).unwrap();
        assert_eq!(approved.state, "approved");
        assert!(approved.device_id.is_some());
        assert_eq!(
            store
                .approve_pair(&id, &initial.verification_code)
                .unwrap()
                .device_id,
            approved.device_id
        );
        assert_eq!(
            store.pair_status(&id, &key).unwrap().device_id,
            approved.device_id
        );
        assert_eq!(store.pending_pairs().unwrap().len(), 0);
        assert_eq!(store.list_devices().unwrap().len(), 1);
        drop(store);
        let resumed = Store::open(dir.path().join("state.sqlite")).unwrap();
        assert_eq!(
            resumed.pair_status(&id, &key).unwrap().device_id,
            approved.device_id
        );
    }

    #[test]
    fn expired_request_keeps_original_key_and_renews_code() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("db")).unwrap();
        let id = Uuid::new_v4().to_string();
        let key = [7u8; 32];
        store
            .request_pair(&id, &key, "L", "linux", "x86_64", "1")
            .unwrap();
        let conn = store.connection().unwrap();
        conn.execute(
            "UPDATE pair_requests SET expires_at_ms=0 WHERE request_id=?1",
            params![id],
        )
        .unwrap();
        assert_eq!(store.pair_status(&id, &key).unwrap().state, "expired");
        assert!(matches!(
            store.approve_pair(&id, "123456"),
            Err(StoreError::PairMismatch)
        ));
        let renewed = store
            .request_pair(&id, &key, "L", "linux", "x86_64", "1")
            .unwrap();
        assert_eq!(renewed.state, "pending");
        assert!(renewed.expires_at_unix_ms > now_ms());
        let approved = store.approve_pair(&id, &renewed.verification_code).unwrap();
        assert_eq!(approved.state, "approved");
        assert_eq!(store.list_devices().unwrap().len(), 1);
    }

    #[test]
    fn bounded_pair_request_flood() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("db")).unwrap();
        for index in 0..MAX_RECENT_REQUESTS {
            let mut key = [0u8; 32];
            key[0] = index as u8;
            let id = Uuid::new_v4().to_string();
            store
                .request_pair(&id, &key, "Test", "linux", "x86_64", "1")
                .unwrap();
        }
        let overflow = store.request_pair(
            &Uuid::new_v4().to_string(),
            &[99u8; 32],
            "Test",
            "linux",
            "x86_64",
            "1",
        );
        assert!(matches!(overflow, Err(StoreError::PairCapacity)));
    }

    #[test]
    fn denied_request_cannot_be_approved() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("state.sqlite")).unwrap();
        let id = Uuid::new_v4().to_string();
        let view = store
            .request_pair(&id, &[4u8; 32], "Linux", "linux", "aarch64", "0")
            .unwrap();
        assert!(store.reject_pair(&id).unwrap());
        assert!(matches!(
            store.approve_pair(&id, &view.verification_code),
            Err(StoreError::PairInvalid)
        ));
        assert_eq!(store.list_devices().unwrap().len(), 0);
    }
}
