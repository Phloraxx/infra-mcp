use serde::Serialize;
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Serialize)]
pub struct CommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

pub fn run(
    command: &mut Command,
    timeout: Duration,
    limit: usize,
) -> Result<CommandOutput, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start {:?}: {error}", command.get_program()))?;
    let stdout = child.stdout.take().ok_or("stdout pipe missing")?;
    let stderr = child.stderr.take().ok_or("stderr pipe missing")?;
    let stdout_thread = thread::spawn(move || read_bounded(stdout, limit));
    let stderr_thread = thread::spawn(move || read_bounded(stderr, limit));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait().map_err(|error| error.to_string())?;
                let (stdout, stdout_truncated) = stdout_thread
                    .join()
                    .map_err(|_| "stdout reader panicked".to_string())?;
                let (stderr, stderr_truncated) = stderr_thread
                    .join()
                    .map_err(|_| "stderr reader panicked".to_string())?;
                return Err(format!(
                    "command timed out after {}ms; stdout={} stderr={} truncated={}",
                    timeout.as_millis(),
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                    stdout_truncated || stderr_truncated
                ));
            }
            Err(error) => return Err(format!("failed waiting for command: {error}")),
        }
    };
    let (stdout, stdout_truncated) = stdout_thread
        .join()
        .map_err(|_| "stdout reader panicked".to_string())?;
    let (stderr, stderr_truncated) = stderr_thread
        .join()
        .map_err(|_| "stderr reader panicked".to_string())?;
    Ok(CommandOutput {
        exit_code: status.code(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        truncated: stdout_truncated || stderr_truncated,
    })
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

fn read_bounded<R: Read>(mut reader: R, limit: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        if kept.len() < limit {
            let take = count.min(limit - kept.len());
            kept.extend_from_slice(&buffer[..take]);
            truncated |= take < count;
        } else {
            truncated = true;
        }
    }
    (kept, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_direct_command_runs_without_shell() {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("cmd.exe");
            command.args(["/D", "/C", "echo infra-runner"]);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = Command::new("printf");
            command.arg("infra-runner");
            command
        };
        let output =
            require_success(run(&mut command, Duration::from_secs(2), 1024).unwrap()).unwrap();
        assert!(output.stdout.contains("infra-runner"));
    }
}
