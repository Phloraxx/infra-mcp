use anyhow::{Context, anyhow};
use serde::Serialize;
#[cfg(not(windows))]
use std::fs;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "macos")]
use std::process::Stdio;

#[cfg(target_os = "macos")]
const LABEL: &str = "io.phloraxx.infra-agent";
#[cfg(windows)]
const WINDOWS_TASK: &str = "Infra MCP Agent";

#[derive(Debug, Clone, Serialize)]
pub struct StartupStatus {
    pub mechanism: &'static str,
    pub installed: bool,
    pub active: bool,
    pub reliable: bool,
    pub detail: String,
}

pub fn sibling_agent_binary() -> anyhow::Result<PathBuf> {
    let current = std::env::current_exe().context("resolve infra CLI path")?;
    let parent = current
        .parent()
        .ok_or_else(|| anyhow!("infra CLI path has no parent"))?;
    #[cfg(windows)]
    let name = "infra-agent.exe";
    #[cfg(not(windows))]
    let name = "infra-agent";
    Ok(parent.join(name))
}

#[cfg(target_os = "linux")]
pub fn install(agent: &Path, state_dir: &Path) -> anyhow::Result<()> {
    let path = linux_unit_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("unit path has no parent"))?;
    fs::create_dir_all(parent)?;
    fs::write(&path, render_systemd_unit(agent, state_dir))?;
    command_ok(user_systemctl().args(["daemon-reload"]))?;
    command_ok(user_systemctl().args(["enable", "infra-agent.service"]))?;
    // `enable --now` is a no-op for an already-running service. Always restart so an
    // idempotent reinstall/repair actually switches to the binary that was just installed.
    command_ok(user_systemctl().args(["restart", "infra-agent.service"]))?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn install(agent: &Path, state_dir: &Path) -> anyhow::Result<()> {
    let path = macos_plist_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("plist path has no parent"))?;
    fs::create_dir_all(parent)?;
    fs::write(&path, render_launchd_plist(agent, state_dir))?;
    let domain = format!("gui/{}", current_uid()?);
    let _ = Command::new("launchctl")
        .args(["bootout", &domain, path.to_string_lossy().as_ref()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    command_ok(Command::new("launchctl").args([
        "bootstrap",
        &domain,
        path.to_string_lossy().as_ref(),
    ]))?;
    command_ok(Command::new("launchctl").args(["enable", &format!("{domain}/{LABEL}")]))?;
    Ok(())
}

#[cfg(windows)]
pub fn install(agent: &Path, state_dir: &Path) -> anyhow::Result<()> {
    // schtasks /TR is ambiguous for quoted executable + arguments when paths
    // contain spaces: Task Scheduler may preserve the outer quotes inside its
    // *Execute* field and fail to launch the program. Supply structured fields
    // through the built-in Windows ScheduledTasks PowerShell module instead.
    let executable = powershell_single_quote(&agent.to_string_lossy());
    let arguments = powershell_single_quote(&format!(
        "--state-dir \"{}\"",
        state_dir.display().to_string().replace('"', "")
    ));
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         $action=New-ScheduledTaskAction -Execute '{executable}' -Argument '{arguments}'; \
         $logon=New-ScheduledTaskTrigger -AtLogOn; \
         $retry=New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) -RepetitionInterval (New-TimeSpan -Minutes 5) -RepetitionDuration (New-TimeSpan -Days 3650); \
         $settings=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Seconds 0) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable; \
         $principal=New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited; \
         Register-ScheduledTask -TaskName '{WINDOWS_TASK}' -Action $action -Trigger @($logon,$retry) -Settings $settings -Principal $principal -Force | Out-Null"
    );
    command_ok(Command::new("powershell.exe").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &script,
    ]))
    .map_err(|error| {
        if error.to_string().to_ascii_lowercase().contains("access is denied") {
            anyhow!(
                "Infra Scheduled Task is administrator-owned or task permissions are restricted. Run infra repair once in Administrator PowerShell; enrollment and running agent were not changed: {error}"
            )
        } else {
            error
        }
    })?;
    // /Run leaves already-running tasks and their durable work alone.
    command_ok(Command::new("schtasks").args(["/Run", "/TN", WINDOWS_TASK]))?;
    Ok(())
}

#[cfg(windows)]
fn powershell_single_quote(value: &str) -> String {
    value.replace(char::from(39), "''")
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> anyhow::Result<()> {
    let _ = user_systemctl()
        .args(["disable", "--now", "infra-agent.service"])
        .status();
    let path = linux_unit_path()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    let _ = user_systemctl().args(["daemon-reload"]).status();
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> anyhow::Result<()> {
    let path = macos_plist_path()?;
    let domain = format!("gui/{}", current_uid()?);
    let _ = Command::new("launchctl")
        .args(["bootout", &domain, path.to_string_lossy().as_ref()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(windows)]
pub fn uninstall() -> anyhow::Result<()> {
    let _ = Command::new("schtasks")
        .args(["/End", "/TN", WINDOWS_TASK])
        .status();
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", WINDOWS_TASK, "/F"])
        .status();
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn restart() -> anyhow::Result<()> {
    command_ok(user_systemctl().args(["restart", "infra-agent.service"]))
}

#[cfg(target_os = "linux")]
pub fn recover_after_failed_start() -> anyhow::Result<()> {
    let _ = user_systemctl()
        .args(["reset-failed", "infra-agent.service"])
        .status();
    restart()
}

#[cfg(target_os = "macos")]
pub fn restart() -> anyhow::Result<()> {
    let target = format!("gui/{}/{LABEL}", current_uid()?);
    command_ok(Command::new("launchctl").args(["kickstart", "-k", &target]))
}

#[cfg(target_os = "macos")]
pub fn recover_after_failed_start() -> anyhow::Result<()> {
    restart()
}

#[cfg(windows)]
pub fn restart() -> anyhow::Result<()> {
    let _ = Command::new("schtasks")
        .args(["/End", "/TN", WINDOWS_TASK])
        .status();
    command_ok(Command::new("schtasks").args(["/Run", "/TN", WINDOWS_TASK]))
}

#[cfg(target_os = "linux")]
pub fn status() -> StartupStatus {
    let installed = linux_unit_path().is_ok_and(|path| path.exists());
    let active = user_systemctl()
        .args(["is-active", "--quiet", "infra-agent.service"])
        .output()
        .is_ok_and(|output| output.status.success());
    StartupStatus {
        mechanism: "systemd-user",
        installed,
        active,
        reliable: installed,
        detail: if active {
            "user service active"
        } else if installed {
            "unit installed but inactive"
        } else {
            "unit not installed"
        }
        .into(),
    }
}

#[cfg(target_os = "macos")]
pub fn status() -> StartupStatus {
    let installed = macos_plist_path().is_ok_and(|path| path.exists());
    let active = current_uid().is_ok_and(|uid| {
        Command::new("launchctl")
            .args(["print", &format!("gui/{uid}/{LABEL}")])
            .output()
            .is_ok_and(|output| output.status.success())
    });
    StartupStatus {
        mechanism: "launchd",
        installed,
        active,
        reliable: installed,
        detail: if active {
            "LaunchAgent loaded"
        } else if installed {
            "plist installed but not loaded"
        } else {
            "plist not installed"
        }
        .into(),
    }
}

#[cfg(windows)]
pub fn status() -> StartupStatus {
    // Query task state and recovery settings once, as typed JSON. Human-readable
    // schtasks output is localized and can contain unrelated "running" text.
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            WINDOWS_STATUS_SCRIPT,
        ])
        .output();
    if let Ok(output) = output
        && output.status.success()
        && let Ok(status) = windows_status_from_json(&output.stdout)
    {
        return status;
    }
    StartupStatus {
        mechanism: "scheduled-task",
        installed: false,
        active: false,
        reliable: false,
        detail: "unable to verify the startup task; it may be missing or inaccessible. Run infra doctor under the intended user account".into(),
    }
}

#[cfg(windows)]
const WINDOWS_STATUS_SCRIPT: &str = r#"
$ErrorActionPreference='Stop'
$t=Get-ScheduledTask -TaskName 'Infra MCP Agent' -ErrorAction Stop
$s=$t.Settings
$logon=@($t.Triggers | Where-Object { $_.CimClass.CimClassName -eq 'MSFT_TaskLogonTrigger' }).Count -gt 0
$retry=@($t.Triggers | Where-Object { $_.CimClass.CimClassName -eq 'MSFT_TaskTimeTrigger' -and $_.Repetition.Interval -eq 'PT5M' }).Count -gt 0
$reliable=[bool]($logon -and $retry -and $s.ExecutionTimeLimit -eq 'PT0S' -and
    $s.RestartCount -ge 1 -and -not $s.DisallowStartIfOnBatteries -and
    -not $s.StopIfGoingOnBatteries -and $s.StartWhenAvailable)
[pscustomobject]@{state=[int]$t.State;reliable=$reliable} | ConvertTo-Json -Compress
"#;

#[cfg(any(windows, test))]
fn windows_status_from_json(bytes: &[u8]) -> serde_json::Result<StartupStatus> {
    #[derive(serde::Deserialize)]
    struct TaskSnapshot {
        state: u32,
        reliable: bool,
    }
    let task: TaskSnapshot = serde_json::from_slice(bytes)?;
    // TASK_STATE_RUNNING=4 is the native Task Scheduler enum value, independent
    // of display language: learn.microsoft.com/windows/win32/api/taskschd/ne-taskschd-task_state
    let active = task.state == 4;
    Ok(StartupStatus {
        mechanism: "scheduled-task",
        installed: true,
        active,
        reliable: task.reliable,
        detail: if !task.reliable {
            "task lacks crash/battery/runtime recovery; run infra repair from Administrator PowerShell if the task is administrator-owned"
        } else if active {
            "scheduled task running with periodic recovery"
        } else {
            "scheduled task installed with recovery but not running"
        }.into(),
    })
}

#[cfg(target_os = "linux")]
fn user_systemctl() -> Command {
    let mut command = Command::new("systemctl");
    command.arg("--user");
    if std::env::var_os("XDG_RUNTIME_DIR").is_none()
        && let Ok(uid) = current_uid_linux()
    {
        let (runtime, bus) = linux_user_bus_values(&uid);
        command.env("XDG_RUNTIME_DIR", runtime);
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            command.env("DBUS_SESSION_BUS_ADDRESS", bus);
        }
    }
    command
}

#[cfg(target_os = "linux")]
fn current_uid_linux() -> anyhow::Result<String> {
    let output = Command::new("id").arg("-u").output()?;
    if !output.status.success() {
        return Err(anyhow!("id -u failed"));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

#[cfg(target_os = "linux")]
fn linux_user_bus_values(uid: &str) -> (String, String) {
    let runtime = format!("/run/user/{uid}");
    let bus = format!("unix:path={runtime}/bus");
    (runtime, bus)
}

#[cfg(target_os = "linux")]
fn linux_unit_path() -> anyhow::Result<PathBuf> {
    Ok(home_dir()?.join(".config/systemd/user/infra-agent.service"))
}

#[cfg(target_os = "macos")]
fn macos_plist_path() -> anyhow::Result<PathBuf> {
    Ok(home_dir()?.join(format!("Library/LaunchAgents/{LABEL}.plist")))
}

#[cfg(not(windows))]
fn home_dir() -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    let key = "USERPROFILE";
    #[cfg(not(windows))]
    let key = "HOME";
    std::env::var_os(key)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("{key} is not set"))
}

#[cfg(target_os = "macos")]
fn current_uid() -> anyhow::Result<String> {
    let output = Command::new("id").arg("-u").output()?;
    if !output.status.success() {
        return Err(anyhow!("id -u failed"));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn command_ok(command: &mut Command) -> anyhow::Result<()> {
    let debug = format!("{command:?}");
    let output = command.output().with_context(|| format!("run {debug}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!("{debug} failed: {}", stderr.trim()))
}

#[cfg(target_os = "linux")]
fn render_systemd_unit(agent: &Path, state_dir: &Path) -> String {
    format!(
        "[Unit]\nDescription=Infra device agent\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStart={} --state-dir {}\n# Keep PTY/job workers alive when only the connectivity controller restarts.\nKillMode=process\nRestart=always\nRestartSec=2\n\n[Install]\nWantedBy=default.target\n",
        systemd_quote(agent),
        systemd_quote(state_dir)
    )
}

#[cfg(target_os = "linux")]
fn systemd_quote(path: &Path) -> String {
    let value = path
        .to_string_lossy()
        .replace('%', "%%")
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{value}\"")
}

#[cfg(target_os = "macos")]
fn render_launchd_plist(agent: &Path, state_dir: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{LABEL}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--state-dir</string><string>{}</string></array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n<key>ProcessType</key><string>Interactive</string>\n</dict></plist>\n",
        xml_escape(&agent.to_string_lossy()),
        xml_escape(&state_dir.to_string_lossy())
    )
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_status_uses_numeric_state_instead_of_localized_text() {
        let running =
            windows_status_from_json(br#"{"state":4,"reliable":true,"label":"En cours"}"#).unwrap();
        assert!(running.installed && running.active && running.reliable);
        let ready = windows_status_from_json(br#"{"state":3,"reliable":true,"description":"running is only a word in this description"}"#).unwrap();
        assert!(ready.installed && !ready.active);
    }

    #[test]
    fn windows_status_keeps_running_separate_from_startup_reliability() {
        let task = windows_status_from_json(br#"{"state":4,"reliable":false}"#).unwrap();
        assert!(task.active);
        assert!(!task.reliable);
        assert!(task.detail.contains("lacks"));
        assert!(windows_status_from_json(br#"{"reliable":true}"#).is_err());
        assert!(windows_status_from_json(b"Running").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_user_bus_values_match_systemd_runtime_layout() {
        let (runtime, bus) = linux_user_bus_values("1001");
        assert_eq!(runtime, "/run/user/1001");
        assert_eq!(bus, "unix:path=/run/user/1001/bus");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_unit_quotes_paths() {
        let unit = render_systemd_unit(
            Path::new("/home/example/Infra Agent/infra-agent"),
            Path::new("/home/example/state dir"),
        );
        assert!(unit.contains("ExecStart=\"/home/example/Infra Agent/infra-agent\" --state-dir \"/home/example/state dir\""));
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("KillMode=process"));
    }

    #[test]
    fn sibling_binary_name_is_platform_specific() {
        let path = sibling_agent_binary().unwrap();
        #[cfg(windows)]
        assert_eq!(path.file_name().unwrap(), "infra-agent.exe");
        #[cfg(not(windows))]
        assert_eq!(path.file_name().unwrap(), "infra-agent");
    }
}
