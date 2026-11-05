use infra_protocol::{
    DEFAULT_READ_LIMIT, FileAction, FileRequest, MAX_DIRECTORY_ENTRIES, MAX_DIRECTORY_PAGE_BYTES,
    MAX_DIRECTORY_PAGE_SIZE, MAX_READ_LIMIT, OperationResult,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_EDIT_FILE_BYTES: u64 = 16 * 1024 * 1024;

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
        Err(FileError::Other(message)) => OperationResult::Failed { error: message },
    }
}

#[derive(Debug)]
enum FileError {
    Other(String),
}

impl From<std::io::Error> for FileError {
    fn from(value: std::io::Error) -> Self {
        Self::Other(value.to_string())
    }
}

fn handle_inner(action: FileAction) -> Result<serde_json::Value, FileError> {
    match action {
        FileAction::Read {
            path,
            offset,
            length,
        } => read_text(&path, offset.unwrap_or(0), bounded_length(length)),
        FileAction::Write {
            path,
            content,
            create_parents,
        } => {
            let path = PathBuf::from(path);
            if create_parents && let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            atomic_write(&path, content.as_bytes())?;
            Ok(serde_json::json!({"ok": true, "path": display(&path), "bytes": content.len()}))
        }
        FileAction::Append {
            path,
            content,
            create_parents,
        } => {
            let path = PathBuf::from(path);
            if create_parents && let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;
            Ok(serde_json::json!({"ok": true, "path": display(&path), "bytes": content.len()}))
        }
        FileAction::ExactEdit {
            path,
            old,
            new,
            expected_replacements,
        } => exact_edit(&PathBuf::from(path), &old, &new, expected_replacements),
        FileAction::Hash { path } => hash_file(&PathBuf::from(path)),
        FileAction::List {
            path,
            offset,
            length,
        } => list(
            &PathBuf::from(path),
            offset.unwrap_or(0),
            length.unwrap_or(MAX_DIRECTORY_PAGE_SIZE),
        ),
        FileAction::Stat { path } => stat(&PathBuf::from(path)),
        FileAction::Mkdir { path } => {
            let path = PathBuf::from(path);
            fs::create_dir_all(&path)?;
            Ok(serde_json::json!({"ok": true, "path": display(&path)}))
        }
        FileAction::Copy {
            source,
            destination,
            recursive,
        } => {
            let source = PathBuf::from(source);
            let destination = PathBuf::from(destination);
            copy_path(&source, &destination, recursive)?;
            Ok(
                serde_json::json!({"ok": true, "source": display(&source), "destination": display(&destination)}),
            )
        }
        FileAction::Move {
            source,
            destination,
        } => {
            let source = PathBuf::from(source);
            let destination = PathBuf::from(destination);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&source, &destination)?;
            Ok(
                serde_json::json!({"ok": true, "source": display(&source), "destination": display(&destination)}),
            )
        }
        FileAction::Trash { path } => trash_path(&PathBuf::from(path)),
        FileAction::DeletePermanent { path, recursive } => {
            let path = PathBuf::from(path);
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                if recursive {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_dir(&path)?;
                }
            } else {
                fs::remove_file(&path)?;
            }
            Ok(serde_json::json!({"ok": true, "path": display(&path), "permanent": true}))
        }
    }
}

fn trash_path(path: &Path) -> Result<serde_json::Value, FileError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    trash::delete(&absolute).map_err(|error| FileError::Other(error.to_string()))?;
    Ok(serde_json::json!({"ok": true, "path": display(&absolute), "trashed": true}))
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

fn exact_edit(
    path: &Path,
    old: &str,
    new: &str,
    expected_replacements: Option<usize>,
) -> Result<serde_json::Value, FileError> {
    if old.is_empty() {
        return Err(FileError::Other("old text cannot be empty".into()));
    }
    let expected = expected_replacements.unwrap_or(1);
    if expected == 0 || expected > 10_000 {
        return Err(FileError::Other(
            "expected_replacements must be between 1 and 10000".into(),
        ));
    }
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_EDIT_FILE_BYTES {
        return Err(FileError::Other(format!(
            "file is too large for exact_edit ({} bytes; max {})",
            metadata.len(),
            MAX_EDIT_FILE_BYTES
        )));
    }
    let bytes = fs::read(path)?;
    if bytes.contains(&0) {
        return Err(FileError::Other(
            "binary file detected; exact_edit refused".into(),
        ));
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| FileError::Other("file is not valid UTF-8; exact_edit refused".into()))?;
    let found = content.match_indices(old).count();
    if found != expected {
        return Err(FileError::Other(format!(
            "exact_edit expected {expected} replacement(s), found {found}; file was not changed"
        )));
    }
    let updated = content.replacen(old, new, expected);
    atomic_write(path, updated.as_bytes())?;
    Ok(serde_json::json!({
        "ok": true,
        "path": display(path),
        "replacements": expected,
        "bytes": updated.len()
    }))
}

fn hash_file(path: &Path) -> Result<serde_json::Value, FileError> {
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest = hasher.finalize();
    let hash = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(serde_json::json!({
        "path": display(path),
        "algorithm": "sha256",
        "sha256": hash,
        "size": size
    }))
}

fn list(path: &Path, offset: u64, length: usize) -> Result<serde_json::Value, FileError> {
    if length == 0 || length > MAX_DIRECTORY_PAGE_SIZE {
        return Err(FileError::Other(format!(
            "list length must be between 1 and {MAX_DIRECTORY_PAGE_SIZE} entries"
        )));
    }
    if offset > MAX_DIRECTORY_ENTRIES as u64 {
        return Err(FileError::Other(format!(
            "list offset must not exceed {MAX_DIRECTORY_ENTRIES} entries"
        )));
    }
    let names = directory_names(path, MAX_DIRECTORY_ENTRIES)?;
    directory_page(
        path,
        &names,
        offset as usize,
        length,
        MAX_DIRECTORY_PAGE_BYTES,
    )
}

fn directory_names(path: &Path, scan_limit: usize) -> Result<Vec<std::ffi::OsString>, FileError> {
    let mut names = Vec::new();
    for item in fs::read_dir(path)? {
        if names.len() == scan_limit {
            return Err(FileError::Other(format!(
                "directory exceeds the {scan_limit}-entry listing scan limit; use bounded search or a narrower directory"
            )));
        }
        names.push(item?.file_name());
    }
    // Sort the whole bounded name set, not an arbitrary prefix from read_dir.
    // Native names retain a deterministic tie-breaker even for lossy UTF-8 display.
    names.sort_unstable();
    Ok(names)
}

fn directory_page(
    path: &Path,
    names: &[std::ffi::OsString],
    offset: usize,
    length: usize,
    byte_limit: usize,
) -> Result<serde_json::Value, FileError> {
    let start = offset.min(names.len());
    let mut entries = Vec::new();
    // Include the fixed envelope/path in the budget; final serialization is
    // checked too. Metadata is read only for the requested page.
    let mut bytes = serde_json::to_vec(&display(path))
        .map_err(|e| FileError::Other(e.to_string()))?
        .len()
        + 512;
    for name in names.iter().skip(start).take(length) {
        let full = path.join(name);
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
        let entry = FileEntry {
            name: name.to_string_lossy().into_owned(),
            path: display(&full),
            kind: kind.into(),
            size: metadata.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
            modified_unix_ms: metadata
                .and_then(|m| m.modified().ok())
                .and_then(system_time_ms),
        };
        let entry_bytes = serde_json::to_vec(&entry)
            .map_err(|e| FileError::Other(e.to_string()))?
            .len()
            + 1;
        if bytes + entry_bytes > byte_limit {
            if entries.is_empty() {
                return Err(FileError::Other(
                    "directory entry exceeds the listing response byte limit; use a narrower path"
                        .into(),
                ));
            }
            break;
        }
        bytes += entry_bytes;
        entries.push(entry);
    }
    let next_offset = start + entries.len();
    let has_more = next_offset < names.len();
    let result = serde_json::json!({
        "path": display(path), "entries": entries, "truncated": has_more,
        "offset": start, "next_offset": next_offset, "has_more": has_more,
        "total_entries": names.len(), "limit": length,
        "pagination_supported": true, "snapshot": false,
        "order": "native_filename_ascending"
    });
    if serde_json::to_vec(&result)
        .map_err(|e| FileError::Other(e.to_string()))?
        .len()
        > byte_limit
    {
        return Err(FileError::Other(
            "directory listing exceeds the response byte limit".into(),
        ));
    }
    Ok(result)
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
    fn directory_pages_cover_more_than_the_old_cap_in_sorted_order() {
        let dir = tempdir().unwrap();
        let expected: Vec<_> = (0..1005).map(|i| format!("entry-{i:04}")).collect();
        for name in expected.iter().rev() {
            fs::write(dir.path().join(name), b"fixture").unwrap();
        }
        let first = list(dir.path(), 0, 1000).unwrap();
        assert_eq!(first["entries"].as_array().unwrap().len(), 1000);
        assert_eq!(first["next_offset"], 1000);
        assert_eq!(first["has_more"], true);
        assert_eq!(first["truncated"], true);
        let second = list(dir.path(), first["next_offset"].as_u64().unwrap(), 1000).unwrap();
        assert_eq!(second["has_more"], false);
        assert_eq!(second["next_offset"], 1005);
        let actual: Vec<_> = [first, second]
            .iter()
            .flat_map(|page| page["entries"].as_array().unwrap())
            .map(|entry| entry["name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(actual, expected);
        let past_end = list(dir.path(), 2000, 10).unwrap();
        assert_eq!(past_end["entries"], serde_json::json!([]));
        assert_eq!(past_end["next_offset"], 1005);
        assert_eq!(past_end["has_more"], false);
    }

    #[test]
    fn directory_scan_and_input_limits_fail_explicitly() {
        let dir = tempdir().unwrap();
        for name in ["A", "B", "a2", "z"] {
            fs::write(dir.path().join(name), b"").unwrap();
        }
        assert!(directory_names(dir.path(), 3).is_err());
        let page = list(dir.path(), 1, 2).unwrap();
        let names: Vec<_> = page["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["B", "a2"]);
        assert!(list(dir.path(), 0, 0).is_err());
        assert!(list(dir.path(), 0, MAX_DIRECTORY_PAGE_SIZE + 1).is_err());
        assert!(list(dir.path(), u64::MAX, 1).is_err());
        let empty = tempdir().unwrap();
        assert_eq!(list(empty.path(), 0, 1).unwrap()["has_more"], false);
    }

    #[test]
    fn byte_limited_directory_pages_continue_without_skipping_entries() {
        let dir = tempdir().unwrap();
        for i in 0..20 {
            fs::write(dir.path().join(format!("{i:02}-{}", "x".repeat(60))), b"").unwrap();
        }
        let names = directory_names(dir.path(), MAX_DIRECTORY_ENTRIES).unwrap();
        let mut offset = 0;
        let mut collected = Vec::new();
        loop {
            let page = directory_page(dir.path(), &names, offset, 20, 1400).unwrap();
            assert!(serde_json::to_vec(&page).unwrap().len() <= 1400);
            let entries = page["entries"].as_array().unwrap();
            assert!(entries.len() < 20);
            collected.extend(
                entries
                    .iter()
                    .map(|entry| entry["name"].as_str().unwrap().to_owned()),
            );
            let next = page["next_offset"].as_u64().unwrap() as usize;
            assert!(next > offset);
            offset = next;
            if !page["has_more"].as_bool().unwrap() {
                break;
            }
        }
        assert_eq!(
            collected,
            names
                .iter()
                .map(|name| name.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_pages_preserve_symlinks() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("z-valid"), b"fixture").unwrap();
        symlink("missing-target", dir.path().join("link")).unwrap();
        let first = list(dir.path(), 0, 1).unwrap();
        assert_eq!(first["entries"][0]["kind"], "symlink");
        let second = list(dir.path(), 1, 1).unwrap();
        assert_eq!(second["entries"][0]["kind"], "file");
        assert_eq!(second["has_more"], false);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn lossy_display_names_still_have_distinct_pagination_positions() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempdir().unwrap();
        for byte in [0xfe, 0xff] {
            fs::write(
                dir.path()
                    .join(std::ffi::OsString::from_vec(vec![b'z', byte])),
                b"fixture",
            )
            .unwrap();
        }
        let first = list(dir.path(), 0, 1).unwrap();
        let second = list(dir.path(), 1, 1).unwrap();
        assert_eq!(first["entries"][0]["name"], second["entries"][0]["name"]);
        assert_eq!(first["has_more"], true);
        assert_eq!(second["has_more"], false);
        assert_eq!(second["next_offset"], 2);
    }

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

    #[test]
    fn exact_edit_requires_expected_match_count() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("edit.txt");
        fs::write(&file, "alpha beta alpha").unwrap();
        assert!(exact_edit(&file, "alpha", "omega", Some(1)).is_err());
        exact_edit(&file, "alpha", "omega", Some(2)).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "omega beta omega");
    }

    #[test]
    fn hash_is_sha256() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("hash.txt");
        fs::write(&file, b"abc").unwrap();
        let value = hash_file(&file).unwrap();
        assert_eq!(
            value["sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
