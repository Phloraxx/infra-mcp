use std::{
    path::Path,
    process::{Command, Stdio},
};
use uuid::Uuid;

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
        let child = Command::new(exe)
            .arg(kind.flag())
            .arg(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("start durable worker: {error}"))?;
        Ok(Some(child.id()))
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
