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
    let command = format!(
        "\"{}\" --state-dir \"{}\"",
        agent.display(),
        state_dir.display()
    );
    command_ok(Command::new("schtasks").args([
        "/Create",
        "/SC",
        "ONLOGON",
        "/TN",
        WINDOWS_TASK,
        "/TR",
        &command,
        "/F",
    ]))?;
    // A created task is not proof that the scheduler launched it.
    command_ok(Command::new("schtasks").args(["/Run", "/TN", WINDOWS_TASK]))?;
    Ok(())
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
    let output = Command::new("schtasks")
        .args(["/Query", "/TN", WINDOWS_TASK, "/FO", "LIST"])
        .output();
    let installed = output.as_ref().is_ok_and(|output| output.status.success());
    let text = output
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .unwrap_or_default();
    let active = installed && text.to_ascii_lowercase().contains("running");
    StartupStatus {
        mechanism: "scheduled-task",
        installed,
        active,
        detail: if active {
            "scheduled task running"
        } else if installed {
            "scheduled task installed"
        } else {
            "scheduled task not installed"
        }
        .into(),
    }
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
