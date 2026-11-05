use std::{
    path::Path,
    process::{Command, Stdio},
};
use uuid::Uuid;

#[cfg(not(target_os = "macos"))]
use std::{
    process::Child,
    sync::{OnceLock, mpsc},
    time::Duration,
};

// One sleeping reaper for all durable workers, not one thread per session.
// Waiting for a worker collects its exit status without owning its lifetime:
// controller exit still leaves active workers running under platform policy.
#[cfg(not(target_os = "macos"))]
fn worker_reaper() -> Result<&'static mpsc::Sender<Child>, String> {
    static REAPER: OnceLock<Result<mpsc::Sender<Child>, String>> = OnceLock::new();
    REAPER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Child>();
        std::thread::Builder::new().name("infra-worker-reaper".into()).spawn(move || {
            let mut children: Vec<Child> = Vec::new();
            loop {
                let next = if children.is_empty() {
                    match rx.recv() {
                        Ok(child) => Some(child),
                        Err(_) => break,
                    }
                } else {
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(child) => Some(child),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                };
                if let Some(child) = next { children.push(child); }
                children.retain_mut(|child| match child.try_wait() {
                    Ok(Some(_)) => false,
                    Ok(None) => true,
                    Err(error) => {
                        tracing::warn!(pid=child.id(), %error, "failed to collect durable worker exit");
                        false
                    }
                });
            }
        }).map_err(|error| format!("start worker reaper: {error}"))?;
        Ok(tx)
    }).as_ref().map_err(Clone::clone)
}

#[derive(Debug, Clone, Copy)]
pub enum WorkerKind {
    Job,
    Pty,
}

impl WorkerKind {
    fn flag(self) -> &'static str {
        match self {
            Self::Job => "--job-worker",
            Self::Pty => "--pty-worker",
        }
    }

    #[cfg(target_os = "macos")]
    fn label(self, id: Uuid) -> String {
        let kind = match self {
            Self::Job => "job",
            Self::Pty => "pty",
        };
        format!("io.phloraxx.infra.worker.{kind}.{id}")
    }
}

/// Launch a durable worker outside the controller lifecycle where the platform requires it.
///
/// Linux relies on the controller systemd unit's `KillMode=process`; Windows workers are normal
/// child processes for now. launchd, however, tears down descendants when a LaunchAgent is
/// kickstarted, so macOS workers are submitted as independent launchd jobs.
pub fn spawn(kind: WorkerKind, id: Uuid, dir: &Path) -> Result<Option<u32>, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    #[cfg(not(target_os = "macos"))]
    let _ = id;

    #[cfg(target_os = "macos")]
    {
        let label = kind.label(id);
        // A stale registration should never block recovery/retry of the same persisted worker.
        let _ = Command::new("launchctl")
            .args(["remove", &label])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        // `launchctl submit` keeps a submitted program alive after failure. A PTY worker can
        // legitimately observe a non-zero child exit when the user closes the terminal; launching
        // the agent directly would therefore resurrect a finished session. Run it under a tiny
        // positional-argument wrapper that always removes its own launchd registration after the
        // worker returns. Paths are passed as argv, never interpolated into shell source.
        const WRAPPER: &str =
            r#""$1" "$2" "$3"; /bin/launchctl remove "$4" >/dev/null 2>&1 || true; exit 0"#;
        let output = Command::new("launchctl")
            .args([
                "submit",
                "-l",
                &label,
                "--",
                "/bin/sh",
                "-c",
                WRAPPER,
                "infra-worker",
            ])
            .arg(&exe)
            .arg(kind.flag())
            .arg(dir)
            .arg(&label)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| format!("submit durable {label}: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "submit durable {label}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(None)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let reaper = worker_reaper()?;
        let child = Command::new(exe)
            .arg(kind.flag())
            .arg(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("start durable worker: {error}"))?;
        let pid = child.id();
        reaper
            .send(child)
            .map_err(|_| "durable worker reaper unavailable".to_string())?;
        Ok(Some(pid))
    }
}

pub fn cleanup_registration(kind: WorkerKind, id: Uuid) {
    #[cfg(target_os = "macos")]
    {
        let label = kind.label(id);
        let _ = Command::new("launchctl")
            .args(["remove", &label])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (kind, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_flags_are_stable() {
        assert_eq!(WorkerKind::Job.flag(), "--job-worker");
        assert_eq!(WorkerKind::Pty.flag(), "--pty-worker");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn completed_workers_are_reaped_without_waiting_in_the_controller() {
        let reaper = worker_reaper().unwrap();
        let mut pids = Vec::new();
        for _ in 0..100 {
            let child = Command::new("/bin/sh")
                .args(["-c", "exit 0"])
                .spawn()
                .unwrap();
            pids.push(child.id());
            reaper.send(child).unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while pids
            .iter()
            .any(|pid| Path::new(&format!("/proc/{pid}")).exists())
        {
            assert!(
                std::time::Instant::now() < deadline,
                "finished children were not collected"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launchd_labels_are_scoped_by_kind_and_id() {
        let id = Uuid::nil();
        assert_eq!(
            WorkerKind::Pty.label(id),
            "io.phloraxx.infra.worker.pty.00000000-0000-0000-0000-000000000000"
        );
    }
}
