use crate::{
    processes::terminate_process_tree,
    worker_launch::{self, WorkerKind},
};
use infra_protocol::{
    DEFAULT_READ_LIMIT, JobAction, JobRequest, MAX_READ_LIMIT, OperationResult, ReconcileItem,
    ReconcileKind,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use sysinfo::{Pid, System};
use uuid::Uuid;

const MAX_JOB_OUTPUT: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JobMeta {
    id: Uuid,
    command: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    boot_id: String,
    status: String,
    started_at_unix_ms: u64,
    finished_at_unix_ms: Option<u64>,
    runner_pid: Option<u32>,
    command_pid: Option<u32>,
    exit_code: Option<i32>,
    output_truncated: bool,
}

#[derive(Clone)]
pub struct Jobs {
    root: PathBuf,
    boot_id: String,
}

impl Jobs {
    pub fn new(state_dir: &Path, boot_id: String) -> anyhow::Result<Self> {
        let root = state_dir.join("jobs");
        fs::create_dir_all(&root)?;
        Ok(Self { root, boot_id })
    }

    pub fn handle(&self, request: JobRequest) -> OperationResult {
        match self.handle_inner(request.action) {
            Ok(body) => OperationResult::Ok { body },
            Err(error) => OperationResult::Failed { error },
        }
    }

    fn handle_inner(&self, action: JobAction) -> Result<serde_json::Value, String> {
        match action {
            JobAction::Start {
                command,
                cwd,
                timeout_ms,
            } => self.start(command, cwd, timeout_ms),
            JobAction::List => Ok(serde_json::json!({"jobs": self.list()?})),
            JobAction::Read {
                job_id,
                offset,
                length,
            } => self.read(job_id, offset.unwrap_or(0), bounded_length(length)),
            JobAction::Cancel { job_id } => self.cancel(job_id),
        }
    }

    fn start(
        &self,
        command: String,
        cwd: Option<String>,
        timeout_ms: Option<u64>,
    ) -> Result<serde_json::Value, String> {
        if command.trim().is_empty() || command.len() > 32 * 1024 {
            return Err("command must be 1..32768 bytes".into());
        }
        let id = Uuid::new_v4();
        let dir = self.root.join(id.to_string());
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut meta = JobMeta {
            id,
            command,
            cwd,
            timeout_ms: timeout_ms.map(|v| v.clamp(100, 7 * 24 * 60 * 60 * 1000)),
            boot_id: self.boot_id.clone(),
            status: "starting".into(),
            started_at_unix_ms: now_ms(),
            finished_at_unix_ms: None,
            runner_pid: None,
            command_pid: None,
            exit_code: None,
            output_truncated: false,
        };
        write_meta(&dir, &meta).map_err(|e| e.to_string())?;
        if let Some(pid) = worker_launch::spawn(WorkerKind::Job, id, &dir)? {
            meta.runner_pid = Some(pid);
            write_meta(&dir, &meta).map_err(|e| e.to_string())?;
        }
        Ok(serde_json::json!({"job": public_meta(&meta)}))
    }

    fn list(&self) -> Result<Vec<serde_json::Value>, String> {
        let mut jobs = Vec::new();
        let entries = fs::read_dir(&self.root).map_err(|e| e.to_string())?;
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let Ok(mut meta) = read_meta(&entry.path()) else {
                continue;
            };
            let mut changed = false;
            if matches!(meta.status.as_str(), "starting" | "running")
                && (meta.boot_id != self.boot_id
                    || meta.runner_pid.is_some_and(|pid| !process_alive(pid)))
            {
                meta.status = "interrupted".into();
                meta.finished_at_unix_ms = Some(now_ms());
                changed = true;
            }
            if changed {
                let _ = write_meta(&entry.path(), &meta);
            }
            if !matches!(meta.status.as_str(), "starting" | "running" | "canceling") {
                worker_launch::cleanup_registration(WorkerKind::Job, meta.id);
            }
            jobs.push(public_meta(&meta));
        }
        jobs.sort_by_key(|value| {
            std::cmp::Reverse(value["started_at_unix_ms"].as_u64().unwrap_or(0))
        });
        Ok(jobs)
    }

    fn read(&self, id: Uuid, offset: u64, length: usize) -> Result<serde_json::Value, String> {
        let dir = self.root.join(id.to_string());
        let meta = read_meta(&dir).map_err(|_| "job not found".to_string())?;
        let output = dir.join("output.log");
        let mut file = match fs::File::open(&output) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(
                    serde_json::json!({"job": public_meta(&meta), "offset": 0, "next_offset": 0, "total_bytes": 0, "output": "", "has_more": false}),
                );
            }
            Err(err) => return Err(err.to_string()),
        };
        let size = file.metadata().map_err(|e| e.to_string())?.len();
        let start = offset.min(size);
        file.seek(SeekFrom::Start(start))
            .map_err(|e| e.to_string())?;
        let mut bytes = vec![0_u8; length.min((size - start) as usize)];
        let count = file.read(&mut bytes).map_err(|e| e.to_string())?;
        bytes.truncate(count);
        let next = start + count as u64;
        Ok(serde_json::json!({
            "job": public_meta(&meta),
            "offset": start,
            "next_offset": next,
            "total_bytes": size,
            "output": String::from_utf8_lossy(&bytes),
            "has_more": next < size,
        }))
    }

    fn cancel(&self, id: Uuid) -> Result<serde_json::Value, String> {
        let dir = self.root.join(id.to_string());
        let mut meta = read_meta(&dir).map_err(|_| "job not found".to_string())?;
        if !matches!(meta.status.as_str(), "starting" | "running") {
            return Ok(serde_json::json!({"job": public_meta(&meta), "already_finished": true}));
        }
        fs::write(dir.join("cancel.request"), b"cancel").map_err(|e| e.to_string())?;
        if let Some(pid) = meta.command_pid {
            terminate_process_tree(pid, false).or_else(|_| terminate_process_tree(pid, true))?;
        }
        meta.status = "canceling".into();
        write_meta(&dir, &meta).map_err(|e| e.to_string())?;
        Ok(serde_json::json!({"job": public_meta(&meta)}))
    }

    pub fn reconcile_items(&self) -> Vec<ReconcileItem> {
        self.list()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| {
                let status = value["status"].as_str()?.to_string();
                if !matches!(status.as_str(), "starting" | "running" | "canceling") {
                    return None;
                }
                Some(ReconcileItem {
                    id: Uuid::parse_str(value["id"].as_str()?).ok()?,
                    kind: ReconcileKind::Job,
                    status,
                    boot_id: value["boot_id"].as_str()?.to_string(),
                    started_at_unix_ms: value["started_at_unix_ms"].as_u64()?,
                })
            })
            .collect()
    }
}

pub fn run_worker(dir: &Path) -> anyhow::Result<()> {
    let mut meta = read_meta(dir)?;
    meta.runner_pid = Some(std::process::id());
    meta.status = "running".into();
    write_meta(dir, &meta)?;

    #[cfg(windows)]
    let mut command = {
        let mut c = Command::new("cmd.exe");
        c.args(["/D", "/S", "/C", &meta.command]);
        c
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut c = Command::new("/bin/sh");
        c.args(["-lc", &meta.command]);
        c
    };
    if let Some(cwd) = &meta.cwd {
        command.current_dir(cwd);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    meta.command_pid = Some(child.id());
    write_meta(dir, &meta)?;

    let output_path = dir.join("output.log");
    let spool = Arc::new(Mutex::new(Spool::new(&output_path)?));
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_thread = stdout.map(|reader| spawn_spool(reader, spool.clone()));
    let stderr_thread = stderr.map(|reader| spawn_spool(reader, spool.clone()));

    let deadline = meta
        .timeout_ms
        .map(|ms| Instant::now() + Duration::from_millis(ms));
    let mut canceled = false;
    let mut timed_out = false;
    let status = loop {
        if dir.join("cancel.request").exists() {
            canceled = true;
            let _ = terminate_process_tree(child.id(), false)
                .or_else(|_| terminate_process_tree(child.id(), true));
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            timed_out = true;
            let _ = terminate_process_tree(child.id(), true);
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if let Some(thread) = stdout_thread {
        let _ = thread.join();
    }
    if let Some(thread) = stderr_thread {
        let _ = thread.join();
    }
    let truncated = spool.lock().map(|s| s.truncated).unwrap_or(true);

    meta.status = if canceled {
        "canceled"
    } else if timed_out {
        "deadline_exceeded"
    } else if status.success() {
        "completed"
    } else {
        "failed"
    }
    .into();
    meta.exit_code = status.code();
    meta.finished_at_unix_ms = Some(now_ms());
    meta.output_truncated = truncated;
    write_meta(dir, &meta)?;
    Ok(())
}

struct Spool {
    file: fs::File,
    written: usize,
    truncated: bool,
}
impl Spool {
    fn new(path: &Path) -> std::io::Result<Self> {
        Ok(Self {
            file: fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?,
            written: fs::metadata(path).map(|m| m.len() as usize).unwrap_or(0),
            truncated: false,
        })
    }
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let remaining = MAX_JOB_OUTPUT.saturating_sub(self.written);
        let take = remaining.min(bytes.len());
        if take > 0 {
            self.file.write_all(&bytes[..take])?;
            self.file.flush()?;
            self.written += take;
        }
        if take < bytes.len() {
            self.truncated = true;
        }
        Ok(())
    }
}

fn spawn_spool<R: Read + Send + 'static>(
    mut reader: R,
    spool: Arc<Mutex<Spool>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            if let Ok(mut spool) = spool.lock() {
                let _ = spool.write(&buffer[..count]);
            }
        }
    })
}

fn read_meta(dir: &Path) -> anyhow::Result<JobMeta> {
    Ok(serde_json::from_slice(&fs::read(dir.join("job.json"))?)?)
}
fn write_meta(dir: &Path, meta: &JobMeta) -> anyhow::Result<()> {
    let temp = dir.join(format!(".job.{}.tmp", Uuid::new_v4()));
    fs::write(&temp, serde_json::to_vec_pretty(meta)?)?;
    fs::rename(temp, dir.join("job.json"))?;
    Ok(())
}
fn public_meta(meta: &JobMeta) -> serde_json::Value {
    serde_json::json!({
        "id": meta.id, "command": meta.command, "cwd": meta.cwd, "status": meta.status,
        "boot_id": meta.boot_id, "started_at_unix_ms": meta.started_at_unix_ms,
        "finished_at_unix_ms": meta.finished_at_unix_ms, "runner_pid": meta.runner_pid,
        "command_pid": meta.command_pid, "exit_code": meta.exit_code, "output_truncated": meta.output_truncated,
    })
}
fn bounded_length(length: Option<usize>) -> usize {
    length
        .unwrap_or(DEFAULT_READ_LIMIT)
        .clamp(1, MAX_READ_LIMIT)
}
fn process_alive(pid: u32) -> bool {
    System::new_all().process(Pid::from_u32(pid)).is_some()
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
    fn read_uses_offsets() {
        let dir = tempdir().unwrap();
        let jobs = Jobs::new(dir.path(), "boot".into()).unwrap();
        let id = Uuid::new_v4();
        let job_dir = jobs.root.join(id.to_string());
        fs::create_dir_all(&job_dir).unwrap();
        let meta = JobMeta {
            id,
            command: "echo test".into(),
            cwd: None,
            timeout_ms: None,
            boot_id: "boot".into(),
            status: "completed".into(),
            started_at_unix_ms: 1,
            finished_at_unix_ms: Some(2),
            runner_pid: None,
            command_pid: None,
            exit_code: Some(0),
            output_truncated: false,
        };
        write_meta(&job_dir, &meta).unwrap();
        fs::write(job_dir.join("output.log"), b"0123456789").unwrap();
        let value = jobs.read(id, 3, 4).unwrap();
        assert_eq!(value["output"], "3456");
        assert_eq!(value["next_offset"], 7);
    }

    #[cfg(unix)]
    #[test]
    fn worker_spools_output_and_finishes() {
        let dir = tempdir().unwrap();
        let jobs = Jobs::new(dir.path(), "boot".into()).unwrap();
        let id = Uuid::new_v4();
        let job_dir = jobs.root.join(id.to_string());
        fs::create_dir_all(&job_dir).unwrap();
        let meta = JobMeta {
            id,
            command: "printf 'infra-job-test\\n'".into(),
            cwd: None,
            timeout_ms: Some(5_000),
            boot_id: "boot".into(),
            status: "starting".into(),
            started_at_unix_ms: now_ms(),
            finished_at_unix_ms: None,
            runner_pid: None,
            command_pid: None,
            exit_code: None,
            output_truncated: false,
        };
        write_meta(&job_dir, &meta).unwrap();
        run_worker(&job_dir).unwrap();
        let value = jobs.read(id, 0, 4096).unwrap();
        assert!(
            value["output"]
                .as_str()
                .unwrap_or_default()
                .contains("infra-job-test")
        );
        let final_meta = read_meta(&job_dir).unwrap();
        assert_eq!(final_meta.status, "completed");
        assert_eq!(final_meta.exit_code, Some(0));
    }

    #[test]
    fn stale_boot_is_marked_interrupted() {
        let dir = tempdir().unwrap();
        let jobs = Jobs::new(dir.path(), "new-boot".into()).unwrap();
        let id = Uuid::new_v4();
        let job_dir = jobs.root.join(id.to_string());
        fs::create_dir_all(&job_dir).unwrap();
        let meta = JobMeta {
            id,
            command: "sleep 100".into(),
            cwd: None,
            timeout_ms: None,
            boot_id: "old-boot".into(),
            status: "running".into(),
            started_at_unix_ms: 1,
            finished_at_unix_ms: None,
            runner_pid: Some(u32::MAX),
            command_pid: None,
            exit_code: None,
            output_truncated: false,
        };
        write_meta(&job_dir, &meta).unwrap();
        let listed = jobs.list().unwrap();
        assert_eq!(listed[0]["status"], "interrupted");
    }
}
