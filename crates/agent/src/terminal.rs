use crate::worker_launch::{self, WorkerKind};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use infra_protocol::{
    DEFAULT_READ_LIMIT, MAX_READ_LIMIT, OperationResult, ReconcileItem, ReconcileKind,
    TerminalAction, TerminalRequest,
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use sysinfo::{Pid, System};
use uuid::Uuid;

#[cfg(windows)]
use std::process::{Command, Stdio};

const MAX_PTY_SPOOL: usize = 8 * 1024 * 1024;
const WORKER_STARTUP_GRACE_MS: u64 = 10_000;
const RETAIN_PTY_SPOOL: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionMeta {
    id: Uuid,
    command: String,
    cwd: Option<String>,
    rows: u16,
    cols: u16,
    boot_id: String,
    status: String,
    started_at_unix_ms: u64,
    finished_at_unix_ms: Option<u64>,
    worker_pid: Option<u32>,
    child_pid: Option<u32>,
    control_port: Option<u16>,
    control_token: String,
    base_offset: u64,
    output_truncated: bool,
    exit_code: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ControlRequest {
    Write { token: String, data: String },
    Resize { token: String, rows: u16, cols: u16 },
    Close { token: String },
    Status { token: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct ControlResponse {
    ok: bool,
    error: Option<String>,
}

#[derive(Clone)]
pub struct Terminals {
    root: PathBuf,
    boot_id: String,
}

impl Terminals {
    pub fn new(state_dir: &Path, boot_id: String) -> anyhow::Result<Self> {
        let root = state_dir.join("sessions");
        fs::create_dir_all(&root)?;
        Ok(Self { root, boot_id })
    }

    pub fn handle(&self, request: TerminalRequest) -> OperationResult {
        match self.handle_inner(request.action) {
            Ok(body) => OperationResult::Ok { body },
            Err(error) => OperationResult::Failed { error },
        }
    }

    fn handle_inner(&self, action: TerminalAction) -> Result<serde_json::Value, String> {
        match action {
            TerminalAction::Open {
                command,
                cwd,
                rows,
                cols,
            } => self.open(command, cwd, rows.unwrap_or(24), cols.unwrap_or(80)),
            TerminalAction::List => Ok(serde_json::json!({"sessions": self.list()?})),
            TerminalAction::Read {
                session_id,
                offset,
                length,
            } => self.read(session_id, offset.unwrap_or(0), bounded_length(length)),
            TerminalAction::Write { session_id, data } => {
                self.control(session_id, |token| ControlRequest::Write { token, data })
            }
            TerminalAction::Resize {
                session_id,
                rows,
                cols,
            } => self.control(session_id, |token| ControlRequest::Resize {
                token,
                rows,
                cols,
            }),
            TerminalAction::Close { session_id } => {
                self.control(session_id, |token| ControlRequest::Close { token })
            }
        }
    }

    fn open(
        &self,
        command: String,
        cwd: Option<String>,
        rows: u16,
        cols: u16,
    ) -> Result<serde_json::Value, String> {
        if command.trim().is_empty() || command.len() > 32 * 1024 {
            return Err("command must be 1..32768 bytes".into());
        }
        let id = Uuid::new_v4();
        let dir = self.root.join(id.to_string());
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).map_err(|_| "OS random source unavailable".to_string())?;
        let mut meta = SessionMeta {
            id,
            command,
            cwd,
            rows: rows.clamp(1, 500),
            cols: cols.clamp(1, 1000),
            boot_id: self.boot_id.clone(),
            status: "starting".into(),
            started_at_unix_ms: now_ms(),
            finished_at_unix_ms: None,
            worker_pid: None,
            child_pid: None,
            control_port: None,
            control_token: URL_SAFE_NO_PAD.encode(random),
            base_offset: 0,
            output_truncated: false,
            exit_code: None,
        };
        write_meta(&dir, &meta).map_err(|e| e.to_string())?;
        if let Some(pid) = worker_launch::spawn(WorkerKind::Pty, id, &dir)? {
            meta.worker_pid = Some(pid);
            write_meta(&dir, &meta).map_err(|e| e.to_string())?;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(current) = read_meta(&dir) {
                if current.status == "running" && current.control_port.is_some() {
                    return Ok(serde_json::json!({"session": public_meta(&current)}));
                }
                if current.status == "failed" {
                    return Err("PTY worker failed to start".into());
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err("PTY worker did not become ready".into())
    }

    fn list(&self) -> Result<Vec<serde_json::Value>, String> {
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.root)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            if !entry.path().is_dir() {
                continue;
            }
            let Ok(mut meta) = read_meta(&entry.path()) else {
                continue;
            };
            let startup_expired = meta.status == "starting"
                && meta.worker_pid.is_none()
                && now_ms().saturating_sub(meta.started_at_unix_ms) > WORKER_STARTUP_GRACE_MS;
            let worker_missing = meta.worker_pid.is_some_and(|pid| !process_alive(pid));
            if matches!(meta.status.as_str(), "starting" | "running")
                && (meta.boot_id != self.boot_id || startup_expired || worker_missing)
            {
                meta.status = "interrupted".into();
                meta.finished_at_unix_ms = Some(now_ms());
                let _ = write_meta(&entry.path(), &meta);
            }
            if !matches!(meta.status.as_str(), "starting" | "running") {
                worker_launch::cleanup_registration(WorkerKind::Pty, meta.id);
            }
            sessions.push(public_meta(&meta));
        }
        sessions.sort_by_key(|value| {
            std::cmp::Reverse(value["started_at_unix_ms"].as_u64().unwrap_or(0))
        });
        Ok(sessions)
    }

    fn read(
        &self,
        id: Uuid,
        requested_offset: u64,
        length: usize,
    ) -> Result<serde_json::Value, String> {
        let dir = self.root.join(id.to_string());
        let meta = read_meta(&dir).map_err(|_| "terminal session not found".to_string())?;
        let path = dir.join("output.log");
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(
                    serde_json::json!({"session": public_meta(&meta), "offset": meta.base_offset, "next_offset": meta.base_offset, "base_offset": meta.base_offset, "output": "", "has_more": false, "offset_expired": requested_offset < meta.base_offset}),
                );
            }
            Err(err) => return Err(err.to_string()),
        };
        let local_size = file.metadata().map_err(|e| e.to_string())?.len();
        let end_offset = meta.base_offset.saturating_add(local_size);
        let global_start = requested_offset.max(meta.base_offset).min(end_offset);
        let local_start = global_start.saturating_sub(meta.base_offset);
        file.seek(SeekFrom::Start(local_start))
            .map_err(|e| e.to_string())?;
        let mut bytes = vec![0_u8; length.min((local_size - local_start) as usize)];
        let count = file.read(&mut bytes).map_err(|e| e.to_string())?;
        bytes.truncate(count);
        let next = global_start + count as u64;
        Ok(serde_json::json!({
            "session": public_meta(&meta), "offset": global_start, "next_offset": next,
            "base_offset": meta.base_offset, "end_offset": end_offset,
            "output": String::from_utf8_lossy(&bytes), "has_more": next < end_offset,
            "offset_expired": requested_offset < meta.base_offset,
        }))
    }

    fn control<F>(&self, id: Uuid, build: F) -> Result<serde_json::Value, String>
    where
        F: FnOnce(String) -> ControlRequest,
    {
        let dir = self.root.join(id.to_string());
        let meta = read_meta(&dir).map_err(|_| "terminal session not found".to_string())?;
        if meta.boot_id != self.boot_id {
            return Err("terminal belongs to a previous machine boot".into());
        }
        let port = meta
            .control_port
            .ok_or_else(|| "terminal control endpoint is not ready".to_string())?;
        let mut stream = TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}")
                .parse()
                .map_err(|_| "invalid control address")?,
            Duration::from_secs(2),
        )
        .map_err(|e| format!("connect terminal worker: {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
        let request = build(meta.control_token.clone());
        serde_json::to_writer(&mut stream, &request).map_err(|e| e.to_string())?;
        stream.write_all(b"\n").map_err(|e| e.to_string())?;
        stream.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        let response: ControlResponse = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if response.ok {
            Ok(serde_json::json!({"ok": true, "session_id": id}))
        } else {
            Err(response
                .error
                .unwrap_or_else(|| "terminal worker rejected request".into()))
        }
    }

    pub fn reconcile_items(&self) -> Vec<ReconcileItem> {
        self.list()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| {
                let status = value["status"].as_str()?.to_string();
                if !matches!(status.as_str(), "starting" | "running") {
                    return None;
                }
                Some(ReconcileItem {
                    id: Uuid::parse_str(value["id"].as_str()?).ok()?,
                    kind: ReconcileKind::Terminal,
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
    meta.worker_pid = Some(std::process::id());
    let pty = native_pty_system();
    let pair = pty.openpty(PtySize {
        rows: meta.rows,
        cols: meta.cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    #[cfg(windows)]
    let mut command = {
        let mut c = CommandBuilder::new("cmd.exe");
        c.args(["/D", "/S", "/C", &meta.command]);
        c
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut c = CommandBuilder::new("/bin/sh");
        c.args(["-lc", &meta.command]);
        c
    };
    if let Some(cwd) = &meta.cwd {
        command.cwd(cwd);
    }
    let mut child = pair.slave.spawn_command(command)?;
    drop(pair.slave);
    meta.child_pid = child.process_id();
    let reader = pair.master.try_clone_reader()?;
    let writer = Arc::new(Mutex::new(pair.master.take_writer()?));
    let master = Arc::new(Mutex::new(pair.master));
    let killer = Arc::new(Mutex::new(child.clone_killer()));
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    meta.control_port = Some(listener.local_addr()?.port());
    meta.status = "running".into();
    write_meta(dir, &meta)?;

    let alive = Arc::new(AtomicBool::new(true));
    let close_requested = Arc::new(AtomicBool::new(false));
    let control_alive = alive.clone();
    let control_close = close_requested.clone();
    let token = meta.control_token.clone();
    let control_writer = writer.clone();
    let control_master = master.clone();
    let control_killer = killer.clone();
    let control_child_pid = meta.child_pid;
    let control_thread = std::thread::spawn(move || {
        while control_alive.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => handle_control(
                    stream,
                    &token,
                    &control_writer,
                    &control_master,
                    &control_killer,
                    control_child_pid,
                    &control_close,
                ),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(30))
                }
                Err(_) => break,
            }
        }
    });

    let output_path = dir.join("output.log");
    let meta_path = dir.to_path_buf();
    // The spooler only needs temporary access to terminal input when ConPTY emits a startup
    // query. It must not own the writer permanently, otherwise that input handle can keep the
    // pseudo-console alive after the child exits.
    let spool_writer = Arc::downgrade(&writer);
    let spool_thread =
        std::thread::spawn(move || spool_pty(reader, &output_path, &meta_path, spool_writer));
    let status = child.wait()?;
    alive.store(false, Ordering::Relaxed);
    let _ = control_thread.join();
    // Close controller-owned PTY handles before waiting for output EOF. This is required by
    // ConPTY; keeping the master/input side alive can leave the reader blocked indefinitely.
    drop(master);
    drop(writer);
    let _ = spool_thread.join();
    let mut latest = read_meta(dir)?;
    latest.status = if close_requested.load(Ordering::Relaxed) {
        "closed"
    } else if status.success() {
        "completed"
    } else {
        "failed"
    }
    .into();
    latest.exit_code = Some(status.exit_code());
    latest.finished_at_unix_ms = Some(now_ms());
    write_meta(dir, &latest)?;
    Ok(())
}

fn handle_control(
    mut stream: TcpStream,
    token: &str,
    writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    master: &Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
    killer: &Arc<Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    child_pid: Option<u32>,
    close_requested: &Arc<AtomicBool>,
) {
    let response = match read_control_request(&mut stream) {
        Err(error) => Err(error),
        Ok(ControlRequest::Write {
            token: supplied,
            data,
        }) if supplied == token => writer
            .lock()
            .map_err(|_| "writer lock poisoned".to_string())
            .and_then(|mut writer| {
                writer
                    .write_all(data.as_bytes())
                    .map_err(|e| e.to_string())?;
                writer.flush().map_err(|e| e.to_string())
            }),
        Ok(ControlRequest::Resize {
            token: supplied,
            rows,
            cols,
        }) if supplied == token => master
            .lock()
            .map_err(|_| "PTY lock poisoned".to_string())
            .and_then(|master| {
                master
                    .resize(PtySize {
                        rows: rows.clamp(1, 500),
                        cols: cols.clamp(1, 1000),
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .map_err(|e| e.to_string())
            }),
        Ok(ControlRequest::Close { token: supplied }) if supplied == token => {
            close_requested.store(true, Ordering::Relaxed);
            close_terminal_child(killer, child_pid)
        }
        Ok(ControlRequest::Status { token: supplied }) if supplied == token => Ok(()),
        Ok(_) => Err("invalid terminal capability token".into()),
    };
    let reply = match response {
        Ok(()) => ControlResponse {
            ok: true,
            error: None,
        },
        Err(error) => ControlResponse {
            ok: false,
            error: Some(error),
        },
    };
    let _ = serde_json::to_writer(&mut stream, &reply);
    let _ = stream.write_all(b"\n");
    let _ = stream.flush();
}

fn read_control_request(stream: &mut TcpStream) -> Result<ControlRequest, String> {
    // The listening socket is nonblocking so the worker can observe shutdown. On BSD/macOS,
    // an accepted socket may inherit O_NONBLOCK, which can surface as EAGAIN before the local
    // client request bytes arrive. Normalize each accepted connection back to bounded blocking
    // I/O; the timeouts prevent a local process from pinning the single control thread forever.
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("configure control socket blocking mode: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| format!("configure control read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| format!("configure control write timeout: {error}"))?;

    let mut line = String::new();
    let count = BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| format!("read control request: {error}"))?;
    if count == 0 {
        return Err("empty terminal control request".into());
    }
    serde_json::from_str(&line).map_err(|error| format!("invalid control request: {error}"))
}

fn close_terminal_child(
    killer: &Arc<Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    child_pid: Option<u32>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = killer;
        let pid = child_pid.ok_or_else(|| "terminal child pid unavailable".to_string())?;
        let output = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| format!("taskkill terminal child: {error}"))?;
        if output.status.success() || !process_alive(pid) {
            Ok(())
        } else {
            Err(format!(
                "taskkill terminal child failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    #[cfg(not(windows))]
    {
        let _ = child_pid;
        killer
            .lock()
            .map_err(|_| "killer lock poisoned".to_string())?
            .kill()
            .map_err(|error| error.to_string())
    }
}

fn spool_pty(
    mut reader: Box<dyn Read + Send>,
    output_path: &Path,
    session_dir: &Path,
    writer: Weak<Mutex<Box<dyn Write + Send>>>,
) {
    let mut buffer = [0_u8; 8192];
    #[cfg(windows)]
    let mut pending = Vec::new();

    while let Ok(count) = reader.read(&mut buffer) {
        if count == 0 {
            break;
        }

        #[cfg(windows)]
        {
            let (visible, replies) = filter_terminal_queries(&mut pending, &buffer[..count]);
            for reply in replies {
                let Some(writer) = writer.upgrade() else {
                    return;
                };
                let Ok(mut input) = writer.lock() else {
                    return;
                };
                if input.write_all(&reply).and_then(|_| input.flush()).is_err() {
                    return;
                }
            }
            if !visible.is_empty()
                && append_and_compact(output_path, session_dir, &visible).is_err()
            {
                break;
            }
        }

        #[cfg(not(windows))]
        if append_and_compact(output_path, session_dir, &buffer[..count]).is_err() {
            break;
        }
    }

    #[cfg(windows)]
    if !pending.is_empty() {
        let _ = append_and_compact(output_path, session_dir, &pending);
    }

    #[cfg(not(windows))]
    let _ = writer;
}

/// ConPTY is a pseudo-console, not a byte-transparent pipe. During startup it can ask the
/// terminal for status/cursor/device attributes and wait for a reply before delivering normal
/// interactive input. Infra does not need a full terminal emulator for these handshakes, but it
/// does need to answer them and keep the query bytes out of the transcript.
#[cfg(any(windows, test))]
fn filter_terminal_queries(pending: &mut Vec<u8>, input: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    const QUERIES: &[(&[u8], &[u8])] = &[
        (b"\x1b[5n", b"\x1b[0n"),
        (b"\x1b[6n", b"\x1b[1;1R"),
        (b"\x1b[c", b"\x1b[?1;2c"),
        (b"\x1b[0c", b"\x1b[?1;2c"),
    ];

    pending.extend_from_slice(input);
    let data = std::mem::take(pending);
    let mut visible = Vec::with_capacity(data.len());
    let mut replies = Vec::new();
    let mut index = 0;

    while index < data.len() {
        if let Some((query, reply)) = QUERIES
            .iter()
            .find(|(query, _)| data[index..].starts_with(query))
        {
            replies.push(reply.to_vec());
            index += query.len();
            continue;
        }

        let remainder = &data[index..];
        if QUERIES
            .iter()
            .any(|(query, _)| query.starts_with(remainder) && remainder.len() < query.len())
        {
            pending.extend_from_slice(remainder);
            break;
        }

        visible.push(data[index]);
        index += 1;
    }

    (visible, replies)
}

fn append_and_compact(output_path: &Path, session_dir: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(output_path)?;
        file.write_all(bytes)?;
        file.flush()?;
    }
    let size = fs::metadata(output_path)?.len() as usize;
    if size <= MAX_PTY_SPOOL {
        return Ok(());
    }
    let all = fs::read(output_path)?;
    let keep_from = all.len().saturating_sub(RETAIN_PTY_SPOOL);
    let removed = keep_from as u64;
    let temp = session_dir.join(format!(".output.{}.tmp", Uuid::new_v4()));
    fs::write(&temp, &all[keep_from..])?;
    fs::rename(temp, output_path)?;
    let mut meta = read_meta(session_dir)?;
    meta.base_offset = meta.base_offset.saturating_add(removed);
    meta.output_truncated = true;
    write_meta(session_dir, &meta)?;
    Ok(())
}

fn read_meta(dir: &Path) -> anyhow::Result<SessionMeta> {
    Ok(serde_json::from_slice(&fs::read(
        dir.join("session.json"),
    )?)?)
}
fn write_meta(dir: &Path, meta: &SessionMeta) -> anyhow::Result<()> {
    let temp = dir.join(format!(".session.{}.tmp", Uuid::new_v4()));
    fs::write(&temp, serde_json::to_vec_pretty(meta)?)?;
    fs::rename(temp, dir.join("session.json"))?;
    Ok(())
}
fn public_meta(meta: &SessionMeta) -> serde_json::Value {
    serde_json::json!({"id": meta.id, "command": meta.command, "cwd": meta.cwd, "rows": meta.rows, "cols": meta.cols, "boot_id": meta.boot_id, "status": meta.status, "started_at_unix_ms": meta.started_at_unix_ms, "finished_at_unix_ms": meta.finished_at_unix_ms, "worker_pid": meta.worker_pid, "child_pid": meta.child_pid, "base_offset": meta.base_offset, "output_truncated": meta.output_truncated, "exit_code": meta.exit_code})
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
    fn control_request_normalizes_inherited_nonblocking_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            std::thread::sleep(Duration::from_millis(40));
            serde_json::to_writer(
                &mut stream,
                &ControlRequest::Status {
                    token: "delayed-token".into(),
                },
            )
            .unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
        });
        let (mut server, _) = listener.accept().unwrap();
        server.set_nonblocking(true).unwrap();
        let request = read_control_request(&mut server).unwrap();
        match request {
            ControlRequest::Status { token } => assert_eq!(token, "delayed-token"),
            other => panic!("unexpected control request: {other:?}"),
        }
        client.join().unwrap();
    }

    #[test]
    fn pty_worker_can_write_read_resize_and_close() {
        let dir = tempdir().unwrap();
        let terminals = Terminals::new(dir.path(), "boot".into()).unwrap();
        let id = Uuid::new_v4();
        let session_dir = terminals.root.join(id.to_string());
        fs::create_dir_all(&session_dir).unwrap();
        let meta = SessionMeta {
            id,
            command: if cfg!(windows) {
                // Use an interactive shell rather than `more`: ConPTY console input treats
                // Enter as CR/CRLF, while `more` may buffer LF-only input indefinitely.
                "cmd.exe /D /Q /K".into()
            } else {
                "cat".into()
            },
            cwd: None,
            rows: 24,
            cols: 80,
            boot_id: "boot".into(),
            status: "starting".into(),
            started_at_unix_ms: now_ms(),
            finished_at_unix_ms: None,
            worker_pid: None,
            child_pid: None,
            control_port: None,
            control_token: "test-capability-token".into(),
            base_offset: 0,
            output_truncated: false,
            exit_code: None,
        };
        write_meta(&session_dir, &meta).unwrap();
        let worker_dir = session_dir.clone();
        let (worker_done_tx, worker_done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = worker_done_tx.send(run_worker(&worker_dir));
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let current = read_meta(&session_dir).unwrap();
            if current.status == "running" && current.control_port.is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "PTY worker did not become ready"
            );
            std::thread::sleep(Duration::from_millis(25));
        }

        terminals
            .control(id, |token| ControlRequest::Resize {
                token,
                rows: 30,
                cols: 100,
            })
            .unwrap();
        terminals
            .control(id, |token| ControlRequest::Write {
                token,
                data: if cfg!(windows) {
                    "echo infra-pty-test\r\n".into()
                } else {
                    "infra-pty-test\n".into()
                },
            })
            .unwrap();
        // A cold Windows ConPTY can take longer than Unix PTYs to emit its first frame.
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            let value = terminals.read(id, 0, 4096).unwrap();
            if value["output"]
                .as_str()
                .unwrap_or_default()
                .contains("infra-pty-test")
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "PTY output never arrived; output={:?}",
                value["output"].as_str().unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        terminals
            .control(id, |token| ControlRequest::Close { token })
            .unwrap();
        worker_done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("PTY worker did not stop within 5 seconds after close")
            .unwrap();
        let final_meta = read_meta(&session_dir).unwrap();
        assert_eq!(final_meta.status, "closed");
    }

    #[test]
    fn terminal_queries_are_replied_to_and_filtered_from_output() {
        let mut pending = Vec::new();
        let (visible, replies) = filter_terminal_queries(&mut pending, b"before\x1b[6nafter\x1b[c");
        assert_eq!(visible, b"beforeafter");
        assert_eq!(replies, vec![b"\x1b[1;1R".to_vec(), b"\x1b[?1;2c".to_vec()]);
        assert!(pending.is_empty());
    }

    #[test]
    fn terminal_query_filter_handles_split_escape_sequences() {
        let mut pending = Vec::new();
        let (visible, replies) = filter_terminal_queries(&mut pending, b"hello\x1b[");
        assert_eq!(visible, b"hello");
        assert!(replies.is_empty());
        assert_eq!(pending, b"\x1b[");

        let (visible, replies) = filter_terminal_queries(&mut pending, b"6nworld");
        assert_eq!(visible, b"world");
        assert_eq!(replies, vec![b"\x1b[1;1R".to_vec()]);
        assert!(pending.is_empty());
    }

    #[test]
    fn abandoned_starting_terminal_is_interrupted_after_grace() {
        let dir = tempdir().unwrap();
        let terminals = Terminals::new(dir.path(), "boot".into()).unwrap();
        let id = Uuid::new_v4();
        let session_dir = terminals.root.join(id.to_string());
        fs::create_dir_all(&session_dir).unwrap();
        let meta = SessionMeta {
            id,
            command: "cat".into(),
            cwd: None,
            rows: 24,
            cols: 80,
            boot_id: "boot".into(),
            status: "starting".into(),
            started_at_unix_ms: now_ms().saturating_sub(WORKER_STARTUP_GRACE_MS + 1),
            finished_at_unix_ms: None,
            worker_pid: None,
            child_pid: None,
            control_port: None,
            control_token: "unused-test-token".into(),
            base_offset: 0,
            output_truncated: false,
            exit_code: None,
        };
        write_meta(&session_dir, &meta).unwrap();
        let listed = terminals.list().unwrap();
        assert_eq!(listed[0]["status"], "interrupted");
        assert!(terminals.reconcile_items().is_empty());
    }

    #[test]
    fn stale_boot_is_interrupted() {
        let dir = tempdir().unwrap();
        let terminals = Terminals::new(dir.path(), "new".into()).unwrap();
        let id = Uuid::new_v4();
        let session_dir = terminals.root.join(id.to_string());
        fs::create_dir_all(&session_dir).unwrap();
        let meta = SessionMeta {
            id,
            command: "cat".into(),
            cwd: None,
            rows: 24,
            cols: 80,
            boot_id: "old".into(),
            status: "running".into(),
            started_at_unix_ms: 1,
            finished_at_unix_ms: None,
            worker_pid: Some(u32::MAX),
            child_pid: None,
            control_port: None,
            control_token: "secret".into(),
            base_offset: 0,
            output_truncated: false,
            exit_code: None,
        };
        write_meta(&session_dir, &meta).unwrap();
        let listed = terminals.list().unwrap();
        assert_eq!(listed[0]["status"], "interrupted");
    }
}
