#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use serde::Serialize;
use std::{
    io,
    process::{Command, Stdio},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

/// `script` is an intentional shell program, not a single executable argument.
/// cmd.exe does not decode the C-runtime escaping applied by `Command::arg`.
#[cfg(windows)]
pub fn windows_shell(script: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new("cmd.exe");
    command.args(["/D", "/S", "/C"]);
    // /S removes this outer pair; preserve the caller's internal shell syntax.
    command.raw_arg(format!("\"{script}\""));
    command
}

#[derive(Debug, Serialize)]
pub struct CommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

#[derive(Debug)]
pub enum CommandError {
    DeadlineExceeded(CommandOutput),
    Failed(String),
}

// Dropping an abandoned execution must signal its entire owned group/job, not
// just the shell. Tokio's underlying kill-on-drop also collects the direct child.
struct OwnedChild(Box<dyn ChildWrapper>, bool);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.1 {
            let _ = self.0.start_kill();
        }
    }
}

#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
}

impl Capture {
    fn append(&mut self, bytes: &[u8], limit: usize) {
        let count = bytes.len().min(limit.saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&bytes[..count]);
        self.truncated |= count < bytes.len();
    }
}

async fn capture<R: AsyncRead + Unpin>(
    mut reader: R,
    output: &mut Capture,
    limit: usize,
) -> io::Result<()> {
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok(());
        }
        output.append(&buffer[..count], limit);
    }
}

/// One deadline covers both command completion and pipe EOF. Reader futures are
/// owned here, so timeout/cancellation closes pipes without detached reader tasks.
pub async fn run_async(
    mut command: tokio::process::Command,
    timeout: Duration,
    limit: usize,
) -> Result<CommandOutput, CommandError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut wrapped = CommandWrap::from(command);
    wrapped.wrap(KillOnDrop);
    #[cfg(unix)]
    wrapped.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    wrapped.wrap(JobObject);
    let mut child = OwnedChild(
        wrapped
            .spawn()
            .map_err(|error| CommandError::Failed(format!("spawn failed: {error}")))?,
        true,
    );
    let stdout = child.0.stdout().take().expect("stdout piped");
    let stderr = child.0.stderr().take().expect("stderr piped");
    let mut out = Capture::default();
    let mut err = Capture::default();
    let result = tokio::time::timeout(timeout, async {
        tokio::try_join!(
            child.0.wait(),
            capture(stdout, &mut out, limit),
            capture(stderr, &mut err, limit)
        )
    })
    .await;
    let output = |exit_code, incomplete| CommandOutput {
        exit_code,
        stdout: String::from_utf8_lossy(&out.bytes).into_owned(),
        stderr: String::from_utf8_lossy(&err.bytes).into_owned(),
        truncated: incomplete || out.truncated || err.truncated,
    };
    match result {
        Ok(Ok((status, (), ()))) => {
            child.1 = false;
            Ok(output(status.code(), false))
        }
        other => {
            // start_kill targets the group/job even if its original shell exited.
            let _ = child.0.start_kill();
            let cleanup = tokio::time::timeout(CLEANUP_TIMEOUT, child.0.wait()).await;
            if !matches!(cleanup, Ok(Ok(_))) {
                return Err(CommandError::Failed("command cleanup could not be confirmed after termination; inspect process state before retrying".into()));
            }
            child.1 = false;
            match other {
                Err(_) => Err(CommandError::DeadlineExceeded(output(None, true))),
                Ok(Err(error)) => Err(CommandError::Failed(format!(
                    "command/output wait failed: {error}; inspect state before retrying"
                ))),
                Ok(Ok(_)) => unreachable!(),
            }
        }
    }
}

/// Structured operations call this from spawn_blocking. Plain synchronous tests
/// use a temporary runtime; both paths share the same bounded capture semantics.
pub fn run(command: Command, timeout: Duration, limit: usize) -> Result<CommandOutput, String> {
    let future = run_async(tokio::process::Command::from(command), timeout, limit);
    let result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(future),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("create command runtime: {error}"))?
            .block_on(future),
    };
    match result {
        Ok(output) => Ok(output),
        Err(CommandError::Failed(error)) => Err(error),
        Err(CommandError::DeadlineExceeded(output)) => Err(format!(
            "command timed out after {}ms; stdout={} stderr={} truncated={}; operation was not replayed",
            timeout.as_millis(),
            output.stdout,
            output.stderr,
            output.truncated,
        )),
    }
}

pub fn require_success(output: CommandOutput) -> Result<CommandOutput, String> {
    if output.exit_code == Some(0) {
        Ok(output)
    } else {
        Err(format!(
            "command failed with exit {:?}: {}",
            output.exit_code,
            output.stderr.trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_direct_command_runs_without_shell() {
        #[cfg(windows)]
        let mut command = Command::new("cmd.exe");
        #[cfg(windows)]
        command.args(["/D", "/C", "echo infra-runner"]);
        #[cfg(not(windows))]
        let mut command = Command::new("printf");
        #[cfg(not(windows))]
        command.arg("infra-runner");
        let output = require_success(run(command, Duration::from_secs(2), 1024).unwrap()).unwrap();
        assert!(output.stdout.contains("infra-runner"));
    }

    #[tokio::test]
    async fn bounded_capture_drains_excess_output_without_retaining_it() {
        use tokio::io::AsyncWriteExt;
        let (mut writer, reader) = tokio::io::duplex(4096);
        let producer = tokio::spawn(async move {
            writer.write_all(&vec![b'x'; 16384]).await.unwrap();
        });
        let mut output = Capture::default();
        capture(reader, &mut output, 128).await.unwrap();
        producer.await.unwrap();
        assert_eq!(output.bytes.len(), 128);
        assert!(output.truncated);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inherited_pipes_cannot_outlive_deadline_after_shell_exit() {
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", "printf partial-output; sleep 30 &"]);
        let result = tokio::time::timeout(
            Duration::from_secs(4),
            run_async(command, Duration::from_millis(200), 1024),
        )
        .await
        .expect("inherited pipes must not hang the caller");
        let Err(CommandError::DeadlineExceeded(output)) = result else {
            panic!("expected deadline, got {result:?}")
        };
        assert!(output.stdout.contains("partial-output"));
        assert!(output.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn synchronous_structured_runner_also_bounds_inherited_pipes() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf structured-partial; sleep 30 &"]);
        let start = std::time::Instant::now();
        let error = run(command, Duration::from_millis(200), 1024).unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(4));
        assert!(error.contains("timed out"));
        assert!(error.contains("structured-partial"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_terminates_owned_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("descendant.pid");
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", "sleep 30 & echo $! > \"$1\"; wait", "fixture"]);
        command.arg(&file);
        let caller = tokio::spawn(run_async(command, Duration::from_secs(60), 1024));
        let pid = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(&file)
                    && let Ok(pid) = text.trim().parse::<u32>()
                {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let output = Command::new("ps")
                    .args(["-o", "stat=", "-p", &pid.to_string()])
                    .output()
                    .unwrap();
                let state = String::from_utf8_lossy(&output.stdout);
                if !output.status.success() || state.trim().starts_with('Z') {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("descendant survived cancellation");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_job_deadline_is_bounded() {
        let mut command = tokio::process::Command::new("cmd.exe");
        command.args([
            "/D",
            "/S",
            "/C",
            "echo partial-output & ping -n 30 127.0.0.1 >nul",
        ]);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_async(command, Duration::from_millis(500), 1024),
        )
        .await
        .expect("Windows job must honor its deadline");
        assert!(
            matches!(result, Err(CommandError::DeadlineExceeded(_))),
            "{result:?}"
        );
    }
}
