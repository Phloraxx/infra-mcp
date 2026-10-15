use infra_protocol::{
    DEFAULT_READ_LIMIT, FileAction, FileRequest, MAX_READ_LIMIT, OperationResult,
};
use serde::Serialize;
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_LIST_ENTRIES: usize = 1000;

#[derive(Debug, Serialize)]
struct FileReadOutput {
    path: String,
    content: String,
    offset: u64,
    next_offset: u64,
    size: u64,
    has_more: bool,
}

#[derive(Debug, Serialize)]
struct FileEntry {
    name: String,
    path: String,
    kind: String,
    size: Option<u64>,
    modified_unix_ms: Option<u64>,
}

pub fn handle(request: FileRequest) -> OperationResult {
    match handle_inner(request.action) {
        Ok(body) => OperationResult::Ok { body },
        Err(FileError::Unsupported(message)) => OperationResult::Unsupported { operation: message },
        Err(FileError::Other(message)) => OperationResult::Failed { error: message },
    }
}

#[derive(Debug)]
enum FileError {
    Unsupported(String),
    Other(String),
}

impl From<std::io::Error> for FileError {
    fn from(value: std::io::Error) -> Self {
        Self::Other(value.to_string())
    }
}

fn handle_inner(action: FileAction) -> Result<serde_json::Value, FileError> {
    match action {
        FileAction::Read { path, offset, length } => read_text(&path, offset.unwrap_or(0), bounded_length(length)),
        FileAction::Write { path, content, create_parents } => {
            let path = PathBuf::from(path);
            if create_parents && let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            atomic_write(&path, content.as_bytes())?;
            Ok(serde_json::json!({"ok": true, "path": display(&path), "bytes": content.len()}))
        }
        FileAction::Append { path, content, create_parents } => {
            let path = PathBuf::from(path);
            if create_parents && let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = fs::OpenOptions::new().create(true).append(true).open(&path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;
            Ok(serde_json::json!({"ok": true, "path": display(&path), "bytes": content.len()}))
        }
        FileAction::List { path } => list(&PathBuf::from(path)),
        FileAction::Stat { path } => stat(&PathBuf::from(path)),
        FileAction::Mkdir { path } => {
            let path = PathBuf::from(path);
            fs::create_dir_all(&path)?;
            Ok(serde_json::json!({"ok": true, "path": display(&path)}))
        }
        FileAction::Copy { source, destination, recursive } => {
            let source = PathBuf::from(source);
            let destination = PathBuf::from(destination);
            copy_path(&source, &destination, recursive)?;
            Ok(serde_json::json!({"ok": true, "source": display(&source), "destination": display(&destination)}))
        }
        FileAction::Move { source, destination } => {
            let source = PathBuf::from(source);
            let destination = PathBuf::from(destination);
            if let Some(parent) = destination.parent() { fs::create_dir_all(parent)?; }
            fs::rename(&source, &destination)?;
            Ok(serde_json::json!({"ok": true, "source": display(&source), "destination": display(&destination)}))
        }
        FileAction::Trash { .. } => Err(FileError::Unsupported("trash is not implemented on this platform build; use delete_permanent only when permanent deletion is intended".into())),
        FileAction::DeletePermanent { path, recursive } => {
            let path = PathBuf::from(path);
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                if recursive { fs::remove_dir_all(&path)?; } else { fs::remove_dir(&path)?; }
            } else {
                fs::remove_file(&path)?;
            }
            Ok(serde_json::json!({"ok": true, "path": display(&path), "permanent": true}))
        }
    }
}

fn bounded_length(length: Option<usize>) -> usize {
    length
        .unwrap_or(DEFAULT_READ_LIMIT)
        .clamp(1, MAX_READ_LIMIT)
}

fn read_text(path: &str, offset: u64, length: usize) -> Result<serde_json::Value, FileError> {
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    let start = offset.min(size);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0_u8; length.min((size - start) as usize)];
    let count = file.read(&mut bytes)?;
    bytes.truncate(count);
    if bytes.contains(&0) {
        return Err(FileError::Other("binary file detected; text read refused (binary transfer support is a separate operation)".into()));
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| FileError::Other("file chunk is not valid UTF-8; text read refused".into()))?;
    let next_offset = start.saturating_add(count as u64);
    serde_json::to_value(FileReadOutput {
        path: path.to_string(),
        content,
        offset: start,
        next_offset,
        size,
        has_more: next_offset < size,
    })
    .map_err(|e| FileError::Other(e.to_string()))
}

fn list(path: &Path) -> Result<serde_json::Value, FileError> {
    let mut entries = Vec::new();
    for item in fs::read_dir(path)?.take(MAX_LIST_ENTRIES + 1) {
        let item = item?;
        let full = item.path();
        let metadata = fs::symlink_metadata(&full).ok();
        let kind = metadata
            .as_ref()
            .map(|m| {
                if m.file_type().is_symlink() {
                    "symlink"
                } else if m.is_dir() {
                    "directory"
                } else if m.is_file() {
                    "file"
                } else {
                    "other"
                }
            })
            .unwrap_or("unknown");
        entries.push(FileEntry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: display(&full),
            kind: kind.into(),
            size: metadata.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
            modified_unix_ms: metadata
                .and_then(|m| m.modified().ok())
                .and_then(system_time_ms),
        });
    }
    entries.sort_by_key(|a| a.name.to_lowercase());
    let truncated = entries.len() > MAX_LIST_ENTRIES;
    entries.truncate(MAX_LIST_ENTRIES);
    Ok(serde_json::json!({"path": display(path), "entries": entries, "truncated": truncated}))
}

fn stat(path: &Path) -> Result<serde_json::Value, FileError> {
    let metadata = fs::symlink_metadata(path)?;
    let kind = if metadata.file_type().is_symlink() {
        "symlink"
    } else if metadata.is_dir() {
        "directory"
    } else if metadata.is_file() {
        "file"
    } else {
        "other"
    };
    Ok(serde_json::json!({
        "path": display(path),
        "kind": kind,
        "size": metadata.len(),
        "readonly": metadata.permissions().readonly(),
        "modified_unix_ms": metadata.modified().ok().and_then(system_time_ms),
        "created_unix_ms": metadata.created().ok().and_then(system_time_ms),
    }))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".infra-write-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<(), std::io::Error> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(FileError::from)
}

fn copy_path(source: &Path, destination: &Path, recursive: bool) -> Result<(), FileError> {
    let metadata = fs::metadata(source)?;
    if metadata.is_dir() {
        if !recursive {
            return Err(FileError::Other(
                "source is a directory; recursive=true is required".into(),
            ));
        }
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_path(&entry.path(), &destination.join(entry.file_name()), true)?;
        }
    } else {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn system_time_ms(time: std::time::SystemTime) -> Option<u64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn atomic_write_and_bounded_read() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        atomic_write(&file, b"abcdefghij").unwrap();
        let value = read_text(file.to_str().unwrap(), 2, 4).unwrap();
        assert_eq!(value["content"], "cdef");
        assert_eq!(value["next_offset"], 6);
        assert_eq!(value["has_more"], true);
    }

    #[test]
    fn binary_read_is_refused() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("binary.bin");
        fs::write(&file, [1_u8, 0, 2]).unwrap();
        assert!(read_text(file.to_str().unwrap(), 0, 32).is_err());
    }
}
