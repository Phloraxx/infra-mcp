use base64::{Engine as _, engine::general_purpose::STANDARD};
use infra_protocol::{
    OperationResult, ReconcileItem, ReconcileKind, TransferAction, TransferRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const DEFAULT_CHUNK: usize = 256 * 1024;
const MAX_CHUNK: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TransferMeta {
    id: Uuid,
    direction: String,
    path: String,
    staging_path: Option<String>,
    size: u64,
    sha256: Option<String>,
    overwrite: bool,
    status: String,
    boot_id: String,
    created_at_unix_ms: u64,
    updated_at_unix_ms: u64,
    bytes_transferred: u64,
}

#[derive(Clone)]
pub struct Transfers {
    root: PathBuf,
    boot_id: String,
}

impl Transfers {
    pub fn new(state_dir: &Path, boot_id: String) -> anyhow::Result<Self> {
        let root = state_dir.join("transfers");
        fs::create_dir_all(&root)?;
        Ok(Self { root, boot_id })
    }

    pub fn handle(&self, request: TransferRequest) -> OperationResult {
        match self.handle_inner(request.action) {
            Ok(body) => OperationResult::Ok { body },
            Err(error) => OperationResult::Failed { error },
        }
    }

    fn handle_inner(&self, action: TransferAction) -> Result<serde_json::Value, String> {
        match action {
            TransferAction::UploadBegin {
                path,
                size,
                sha256,
                overwrite,
                create_parents,
            } => self.upload_begin(path, size, sha256, overwrite, create_parents),
            TransferAction::UploadChunk {
                transfer_id,
                offset,
                data_base64,
            } => self.upload_chunk(transfer_id, offset, &data_base64),
            TransferAction::UploadCommit { transfer_id } => self.upload_commit(transfer_id),
            TransferAction::DownloadBegin { path } => self.download_begin(path),
            TransferAction::DownloadChunk {
                transfer_id,
                offset,
                length,
            } => self.download_chunk(transfer_id, offset, length.unwrap_or(DEFAULT_CHUNK)),
            TransferAction::Status { transfer_id } => self.status(transfer_id),
            TransferAction::List => Ok(serde_json::json!({"transfers": self.list()?})),
            TransferAction::Cancel { transfer_id } => self.cancel(transfer_id),
        }
    }

    fn upload_begin(
        &self,
        path: String,
        size: u64,
        sha256: Option<String>,
        overwrite: bool,
        create_parents: bool,
    ) -> Result<serde_json::Value, String> {
        let destination = PathBuf::from(&path);
        if destination.exists() && !overwrite {
            return Err("destination already exists; set overwrite=true to replace it".into());
        }
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        if create_parents {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if !parent.is_dir() {
            return Err("destination parent directory does not exist".into());
        }
        if let Some(expected) = &sha256
            && !valid_sha256(expected)
        {
            return Err("sha256 must be a 64-character hexadecimal digest".into());
        }
        let id = Uuid::new_v4();
        let dir = self.root.join(id.to_string());
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let staging = parent.join(format!(".infra-upload-{id}.part"));
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staging)
            .map_err(|error| format!("create staging file: {error}"))?;
        let meta = TransferMeta {
            id,
            direction: "upload".into(),
            path,
            staging_path: Some(staging.to_string_lossy().into_owned()),
            size,
            sha256: sha256.map(|value| value.to_ascii_lowercase()),
            overwrite,
            status: "uploading".into(),
            boot_id: self.boot_id.clone(),
            created_at_unix_ms: now_ms(),
            updated_at_unix_ms: now_ms(),
            bytes_transferred: 0,
        };
        write_meta(&dir, &meta).map_err(|error| error.to_string())?;
        Ok(serde_json::json!({"transfer": public_meta(&meta), "chunk_size": DEFAULT_CHUNK}))
    }

    fn upload_chunk(
        &self,
        id: Uuid,
        offset: u64,
        data_base64: &str,
    ) -> Result<serde_json::Value, String> {
        let dir = self.root.join(id.to_string());
        let mut meta = read_meta(&dir).map_err(|_| "transfer not found".to_string())?;
        if meta.direction != "upload" || meta.status != "uploading" {
            return Err("transfer is not an active upload".into());
        }
        let data = STANDARD
            .decode(data_base64)
            .map_err(|_| "data_base64 is invalid".to_string())?;
        if data.is_empty() || data.len() > MAX_CHUNK {
            return Err(format!("upload chunk must be 1..{MAX_CHUNK} decoded bytes"));
        }
        let staging = meta
            .staging_path
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| "upload staging path is missing".to_string())?;
        let current = fs::metadata(&staging)
            .map_err(|error| error.to_string())?
            .len();
        if offset > current {
            return Err(format!(
                "chunk offset {offset} is ahead of next offset {current}"
            ));
        }
        if offset < current {
            if offset.saturating_add(data.len() as u64) > current {
                return Err("retry chunk partially overlaps already-written data".into());
            }
            let mut file = fs::File::open(&staging).map_err(|error| error.to_string())?;
            file.seek(SeekFrom::Start(offset))
                .map_err(|error| error.to_string())?;
            let mut existing = vec![0_u8; data.len()];
            file.read_exact(&mut existing)
                .map_err(|error| error.to_string())?;
            if existing != data {
                return Err("retry chunk does not match already-written data".into());
            }
            return Ok(serde_json::json!({
                "transfer": public_meta(&meta),
                "next_offset": current,
                "idempotent_retry": true
            }));
        }
        let next = current.saturating_add(data.len() as u64);
        if next > meta.size {
            return Err("chunk would exceed declared upload size".into());
        }
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&staging)
            .map_err(|error| error.to_string())?;
        file.write_all(&data).map_err(|error| error.to_string())?;
        file.flush().map_err(|error| error.to_string())?;
        meta.bytes_transferred = next;
        meta.updated_at_unix_ms = now_ms();
        write_meta(&dir, &meta).map_err(|error| error.to_string())?;
        Ok(serde_json::json!({
            "transfer": public_meta(&meta),
            "next_offset": next,
            "has_more": next < meta.size
        }))
    }

    fn upload_commit(&self, id: Uuid) -> Result<serde_json::Value, String> {
        let dir = self.root.join(id.to_string());
        let mut meta = read_meta(&dir).map_err(|_| "transfer not found".to_string())?;
        if meta.direction != "upload" || meta.status != "uploading" {
            return Err("transfer is not an active upload".into());
        }
        let staging = meta
            .staging_path
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| "upload staging path is missing".to_string())?;
        let actual_size = fs::metadata(&staging)
            .map_err(|error| error.to_string())?
            .len();
        if actual_size != meta.size {
            return Err(format!(
                "upload is incomplete: expected {} bytes, have {actual_size}",
                meta.size
            ));
        }
        let actual_hash = sha256_file(&staging).map_err(|error| error.to_string())?;
        if let Some(expected) = &meta.sha256
            && *expected != actual_hash
        {
            return Err(format!(
                "SHA-256 mismatch: expected {expected}, got {actual_hash}"
            ));
        }
        let destination = PathBuf::from(&meta.path);
        if destination.exists() && !meta.overwrite {
            return Err("destination appeared after upload began and overwrite=false".into());
        }
        #[cfg(windows)]
        if destination.exists() {
            fs::remove_file(&destination).map_err(|error| error.to_string())?;
        }
        fs::rename(&staging, &destination).map_err(|error| error.to_string())?;
        meta.status = "completed".into();
        meta.sha256 = Some(actual_hash.clone());
        meta.bytes_transferred = actual_size;
        meta.updated_at_unix_ms = now_ms();
        meta.staging_path = None;
        write_meta(&dir, &meta).map_err(|error| error.to_string())?;
        Ok(serde_json::json!({"transfer": public_meta(&meta), "sha256": actual_hash}))
    }

    fn download_begin(&self, path: String) -> Result<serde_json::Value, String> {
        let source = PathBuf::from(&path);
        let metadata = fs::metadata(&source).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("download source must be a regular file".into());
        }
        let hash = sha256_file(&source).map_err(|error| error.to_string())?;
        let id = Uuid::new_v4();
        let dir = self.root.join(id.to_string());
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let meta = TransferMeta {
            id,
            direction: "download".into(),
            path,
            staging_path: None,
            size: metadata.len(),
            sha256: Some(hash),
            overwrite: false,
            status: if metadata.len() == 0 {
                "completed"
            } else {
                "ready"
            }
            .into(),
            boot_id: self.boot_id.clone(),
            created_at_unix_ms: now_ms(),
            updated_at_unix_ms: now_ms(),
            bytes_transferred: 0,
        };
        write_meta(&dir, &meta).map_err(|error| error.to_string())?;
        Ok(serde_json::json!({"transfer": public_meta(&meta), "chunk_size": DEFAULT_CHUNK}))
    }

    fn download_chunk(
        &self,
        id: Uuid,
        offset: u64,
        length: usize,
    ) -> Result<serde_json::Value, String> {
        let dir = self.root.join(id.to_string());
        let mut meta = read_meta(&dir).map_err(|_| "transfer not found".to_string())?;
        if meta.direction != "download" || !matches!(meta.status.as_str(), "ready" | "completed") {
            return Err("transfer is not a readable download".into());
        }
        let mut file = fs::File::open(&meta.path).map_err(|error| error.to_string())?;
        let actual_size = file.metadata().map_err(|error| error.to_string())?.len();
        if actual_size != meta.size {
            return Err("download source changed size after transfer began".into());
        }
        let start = offset.min(meta.size);
        file.seek(SeekFrom::Start(start))
            .map_err(|error| error.to_string())?;
        let requested = length.clamp(1, MAX_CHUNK).min((meta.size - start) as usize);
        let mut bytes = vec![0_u8; requested];
        let count = file.read(&mut bytes).map_err(|error| error.to_string())?;
        bytes.truncate(count);
        let next = start.saturating_add(count as u64);
        meta.bytes_transferred = meta.bytes_transferred.max(next);
        meta.updated_at_unix_ms = now_ms();
        if next == meta.size {
            let current_hash =
                sha256_file(Path::new(&meta.path)).map_err(|error| error.to_string())?;
            if meta.sha256.as_deref() != Some(current_hash.as_str()) {
                meta.status = "source_changed".into();
                meta.updated_at_unix_ms = now_ms();
                write_meta(&dir, &meta).map_err(|error| error.to_string())?;
                return Err(
                    "download source changed after transfer began; restart the transfer".into(),
                );
            }
            meta.status = "completed".into();
        }
        write_meta(&dir, &meta).map_err(|error| error.to_string())?;
        Ok(serde_json::json!({
            "transfer": public_meta(&meta),
            "offset": start,
            "next_offset": next,
            "data_base64": STANDARD.encode(bytes),
            "has_more": next < meta.size
        }))
    }

    fn status(&self, id: Uuid) -> Result<serde_json::Value, String> {
        let meta = read_meta(&self.root.join(id.to_string()))
            .map_err(|_| "transfer not found".to_string())?;
        Ok(serde_json::json!({"transfer": public_meta(&meta)}))
    }

    fn list(&self) -> Result<Vec<serde_json::Value>, String> {
        let mut transfers = Vec::new();
        for entry in fs::read_dir(&self.root)
            .map_err(|error| error.to_string())?
            .flatten()
        {
            if !entry.path().is_dir() {
                continue;
            }
            if let Ok(meta) = read_meta(&entry.path()) {
                transfers.push(public_meta(&meta));
            }
        }
        transfers.sort_by_key(|value| {
            std::cmp::Reverse(value["created_at_unix_ms"].as_u64().unwrap_or(0))
        });
        Ok(transfers)
    }

    fn cancel(&self, id: Uuid) -> Result<serde_json::Value, String> {
        let dir = self.root.join(id.to_string());
        let mut meta = read_meta(&dir).map_err(|_| "transfer not found".to_string())?;
        if let Some(staging) = meta.staging_path.as_deref() {
            let _ = fs::remove_file(staging);
        }
        meta.status = "canceled".into();
        meta.updated_at_unix_ms = now_ms();
        meta.staging_path = None;
        write_meta(&dir, &meta).map_err(|error| error.to_string())?;
        Ok(serde_json::json!({"transfer": public_meta(&meta)}))
    }

    pub fn reconcile_items(&self) -> Vec<ReconcileItem> {
        self.list()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| {
                let status = value["status"].as_str()?.to_string();
                if !matches!(status.as_str(), "uploading" | "ready") {
                    return None;
                }
                Some(ReconcileItem {
                    id: Uuid::parse_str(value["id"].as_str()?).ok()?,
                    kind: ReconcileKind::Transfer,
                    status,
                    boot_id: value["boot_id"].as_str()?.to_string(),
                    started_at_unix_ms: value["created_at_unix_ms"].as_u64()?,
                })
            })
            .collect()
    }
}

fn read_meta(dir: &Path) -> anyhow::Result<TransferMeta> {
    Ok(serde_json::from_slice(&fs::read(
        dir.join("transfer.json"),
    )?)?)
}

fn write_meta(dir: &Path, meta: &TransferMeta) -> anyhow::Result<()> {
    let temp = dir.join(format!(".transfer.{}.tmp", Uuid::new_v4()));
    fs::write(&temp, serde_json::to_vec_pretty(meta)?)?;
    fs::rename(temp, dir.join("transfer.json"))?;
    Ok(())
}

fn public_meta(meta: &TransferMeta) -> serde_json::Value {
    serde_json::json!({
        "id": meta.id,
        "direction": meta.direction,
        "path": meta.path,
        "size": meta.size,
        "sha256": meta.sha256,
        "status": meta.status,
        "boot_id": meta.boot_id,
        "created_at_unix_ms": meta.created_at_unix_ms,
        "updated_at_unix_ms": meta.updated_at_unix_ms,
        "bytes_transferred": meta.bytes_transferred,
    })
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn now_ms() -> u64 {
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
    fn upload_is_resumable_and_idempotent() {
        let dir = tempdir().unwrap();
        let transfers = Transfers::new(dir.path(), "boot".into()).unwrap();
        let destination = dir.path().join("out.bin");
        let content = b"infra-transfer-data";
        let expected = {
            let mut hasher = Sha256::new();
            hasher.update(content);
            hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        let started = transfers
            .upload_begin(
                destination.to_string_lossy().into_owned(),
                content.len() as u64,
                Some(expected.clone()),
                false,
                false,
            )
            .unwrap();
        let id = Uuid::parse_str(started["transfer"]["id"].as_str().unwrap()).unwrap();
        let encoded = STANDARD.encode(content);
        transfers.upload_chunk(id, 0, &encoded).unwrap();
        let retry = transfers.upload_chunk(id, 0, &encoded).unwrap();
        assert_eq!(retry["idempotent_retry"], true);
        transfers.upload_commit(id).unwrap();
        assert_eq!(fs::read(destination).unwrap(), content);
        let status = transfers.status(id).unwrap();
        assert_eq!(status["transfer"]["sha256"], expected);
    }

    #[test]
    fn download_uses_offsets_and_hash() {
        let dir = tempdir().unwrap();
        let transfers = Transfers::new(dir.path(), "boot".into()).unwrap();
        let source = dir.path().join("source.bin");
        fs::write(&source, b"0123456789").unwrap();
        let started = transfers
            .download_begin(source.to_string_lossy().into_owned())
            .unwrap();
        let id = Uuid::parse_str(started["transfer"]["id"].as_str().unwrap()).unwrap();
        let chunk = transfers.download_chunk(id, 3, 4).unwrap();
        assert_eq!(
            STANDARD
                .decode(chunk["data_base64"].as_str().unwrap())
                .unwrap(),
            b"3456"
        );
        assert_eq!(chunk["next_offset"], 7);
        assert!(started["transfer"]["sha256"].as_str().is_some());
    }
}
