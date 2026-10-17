mod startup;

use anyhow::{Context, anyhow};
use infra_local_state::{
    LocalConfig, RuntimeStatus, default_state_dir, identity_path, read_config, read_runtime,
    write_config,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
#[cfg(target_os = "macos")]
use std::process::Stdio;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::Duration,
};
use sysinfo::{Pid, System};
use url::Url;

#[derive(Debug, Serialize)]
struct LocalStatus {
    state_dir: String,
    configured: bool,
    enrolled: bool,
    process_alive: bool,
    startup: startup::StartupStatus,
    config: Option<LocalConfig>,
    runtime: Option<RuntimeStatus>,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    healthy: bool,
    checks: Vec<DoctorCheck>,
}

#[derive(Debug, Serialize)]
struct DoctorCheck {
    name: &'static str,
    status: &'static str,
    detail: String,
    fixable: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("infra: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<ExitCode> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let json = take_flag(&mut args, "--json");
    let state_dir = take_option(&mut args, "--state-dir")
        .map(PathBuf::from)
        .unwrap_or_else(default_state_dir);
    let Some(command) = args.first().cloned() else {
        print_help();
        return Ok(ExitCode::SUCCESS);
    };
    args.remove(0);

    match command.as_str() {
        "status" => {
            ensure_no_args(&args)?;
            let status = collect_status(&state_dir)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                print_status(&status);
            }
            Ok(ExitCode::SUCCESS)
        }
        "doctor" => {
            ensure_no_args(&args)?;
            let report = doctor(&state_dir).await;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_doctor(&report);
            }
            Ok(if report.healthy {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "install" => install_command(&state_dir, args).await,
        "repair" => {
            ensure_no_args(&args)?;
            let previous_runtime = read_runtime(&state_dir).ok().flatten();
            repair(&state_dir)?;
            let runtime = wait_for_online_reconnect(
                &state_dir,
                previous_runtime.as_ref(),
                Duration::from_secs(15),
            )
            .await?;
            println!(
                "Infra startup registration repaired and agent is online{}.",
                runtime
                    .connection_generation
                    .map(|generation| format!(" (generation {generation})"))
                    .unwrap_or_default()
            );
            Ok(ExitCode::SUCCESS)
        }
        "reconnect" => {
            ensure_no_args(&args)?;
            let previous_runtime = read_runtime(&state_dir).ok().flatten();
            startup::restart()?;
            let runtime = wait_for_online_reconnect(
                &state_dir,
                previous_runtime.as_ref(),
                Duration::from_secs(15),
            )
            .await?;
            println!(
                "Infra agent reconnected{}.",
                runtime
                    .connection_generation
                    .map(|generation| format!(" (generation {generation})"))
                    .unwrap_or_default()
            );
            Ok(ExitCode::SUCCESS)
        }
        "update" => update_command(&state_dir, args).await,
        "uninstall" => {
            let remove_state = args.iter().any(|arg| arg == "--remove-state");
            let unknown = args
                .iter()
                .filter(|arg| arg.as_str() != "--remove-state")
                .cloned()
                .collect::<Vec<_>>();
            ensure_no_args(&unknown)?;
            let stopped_workers = cleanup_persistent_workers(&state_dir);
            startup::uninstall()?;
            if remove_state && state_dir.exists() {
                remove_native_identity_credentials(&state_dir)?;
                fs::remove_dir_all(&state_dir)
                    .with_context(|| format!("remove {}", state_dir.display()))?;
            }
            println!(
                "Infra startup registration removed{}; stopped {} persistent worker(s).",
                if remove_state {
                    " and local state deleted"
                } else {
                    ""
                },
                stopped_workers
            );
            Ok(ExitCode::SUCCESS)
        }
        "help" | "--help" | "-h" => {
            print_help();
            Ok(ExitCode::SUCCESS)
        }
        other => Err(anyhow!("unknown command {other:?}; run `infra help`")),
    }
}

async fn install_command(state_dir: &Path, mut args: Vec<String>) -> anyhow::Result<ExitCode> {
    let gateway = take_option(&mut args, "--gateway")
        .ok_or_else(|| anyhow!("infra install requires --gateway <ws://.../agent>"))?;
    let code = take_option(&mut args, "--code");
    let name = take_option(&mut args, "--name");
    let agent = take_option(&mut args, "--agent")
        .map(PathBuf::from)
        .unwrap_or(startup::sibling_agent_binary()?);
    let no_startup = take_flag(&mut args, "--no-startup");
    ensure_no_args(&args)?;
    if !agent.is_file() {
        return Err(anyhow!("agent binary not found at {}", agent.display()));
    }

    let config = LocalConfig::new(gateway, name)?;
    write_config(state_dir, &config)?;

    if !identity_path(state_dir).exists() {
        let code = code.ok_or_else(|| {
            anyhow!("device is not enrolled; provide the one-time dashboard code with --code")
        })?;
        let status = Command::new(&agent)
            .arg("--state-dir")
            .arg(state_dir)
            .arg("--enroll-only")
            .env("INFRA_ENROLLMENT_CODE", code)
            .status()
            .with_context(|| format!("run {} enrollment", agent.display()))?;
        if !status.success() {
            return Err(anyhow!("agent enrollment failed with {status}"));
        }
    }

    if no_startup {
        println!("Infra configured and enrolled. Startup registration skipped.");
        return Ok(ExitCode::SUCCESS);
    }
    startup::install(&agent, state_dir)?;
    println!(
        "Infra installed. The agent will reconnect automatically after login/reboot where supported."
    );
    Ok(ExitCode::SUCCESS)
}

async fn wait_for_online_reconnect(
    state_dir: &Path,
    previous: Option<&RuntimeStatus>,
    timeout: Duration,
) -> anyhow::Result<RuntimeStatus> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(Some(runtime)) = read_runtime(state_dir)
            && runtime.state == "online"
            && process_alive(runtime.pid)
            && runtime_is_newer(previous, &runtime)
        {
            return Ok(runtime);
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "agent restart was requested but it did not return online; run `infra status` or `infra doctor`"
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn runtime_is_newer(previous: Option<&RuntimeStatus>, current: &RuntimeStatus) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    current.pid != previous.pid
        || current.updated_unix_ms > previous.updated_unix_ms
        || match (
            current.connection_generation,
            previous.connection_generation,
        ) {
            (Some(current), Some(previous)) => current > previous,
            (Some(_), None) => true,
            _ => false,
        }
        || match (
            current.last_connected_unix_ms,
            previous.last_connected_unix_ms,
        ) {
            (Some(current), Some(previous)) => current > previous,
            (Some(_), None) => true,
            _ => false,
        }
}

async fn update_command(state_dir: &Path, mut args: Vec<String>) -> anyhow::Result<ExitCode> {
    let repo = take_option(&mut args, "--repo").unwrap_or_else(|| "Phloraxx/infra-mcp".into());
    let version = take_option(&mut args, "--version").unwrap_or_else(|| "latest".into());
    let base_url = take_option(&mut args, "--base-url");
    ensure_no_args(&args)?;

    let config =
        read_config(state_dir)?.ok_or_else(|| anyhow!("cannot update: config.json is missing"))?;
    if !identity_path(state_dir).is_file() {
        return Err(anyhow!("cannot update: device is not enrolled"));
    }
    if config.gateway_url.trim().is_empty() {
        return Err(anyhow!("cannot update: gateway URL is empty"));
    }

    let cli_path = std::env::current_exe().context("resolve current infra CLI path")?;
    let install_dir = cli_path
        .parent()
        .ok_or_else(|| anyhow!("infra CLI path has no parent"))?;
    #[cfg(windows)]
    let agent_path = install_dir.join("infra-agent.exe");
    #[cfg(not(windows))]
    let agent_path = install_dir.join("infra-agent");
    if !agent_path.is_file() {
        return Err(anyhow!(
            "infra-agent is missing at {}",
            agent_path.display()
        ));
    }

    let (asset, extension) = release_asset_descriptor()?;
    let base = base_url.unwrap_or_else(|| {
        if version == "latest" {
            format!("https://github.com/{repo}/releases/latest/download")
        } else {
            format!("https://github.com/{repo}/releases/download/{version}")
        }
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()?;

    let staged_cli = install_dir.join(format!(".infra.update.{}{}", std::process::id(), extension));
    let staged_agent = install_dir.join(format!(
        ".infra-agent.update.{}{}",
        std::process::id(),
        extension
    ));
    download_verified_release_asset(&client, &base, "infra", &asset, extension, &staged_cli)
        .await?;
    if let Err(error) = download_verified_release_asset(
        &client,
        &base,
        "infra-agent",
        &asset,
        extension,
        &staged_agent,
    )
    .await
    {
        let _ = fs::remove_file(&staged_cli);
        return Err(error);
    }
    make_executable_like(&staged_cli, &cli_path)?;
    make_executable_like(&staged_agent, &agent_path)?;

    #[cfg(windows)]
    {
        apply_windows_update(
            state_dir,
            &base,
            &cli_path,
            &agent_path,
            &staged_cli,
            &staged_agent,
        )
        .await
    }
    #[cfg(not(windows))]
    {
        apply_unix_update(
            state_dir,
            &base,
            &cli_path,
            &agent_path,
            &staged_cli,
            &staged_agent,
        )
        .await
    }
}

#[cfg(not(windows))]
async fn apply_unix_update(
    state_dir: &Path,
    base: &str,
    cli_path: &Path,
    agent_path: &Path,
    staged_cli: &Path,
    staged_agent: &Path,
) -> anyhow::Result<ExitCode> {
    let install_dir = cli_path
        .parent()
        .ok_or_else(|| anyhow!("infra CLI path has no parent"))?;
    let rollback_cli = install_dir.join("infra.rollback");
    let rollback_agent = install_dir.join("infra-agent.rollback");
    let _ = fs::remove_file(&rollback_cli);
    let _ = fs::remove_file(&rollback_agent);

    let previous_runtime = read_runtime(state_dir).ok().flatten();
    fs::rename(agent_path, &rollback_agent)
        .with_context(|| format!("stage rollback copy of {}", agent_path.display()))?;
    if let Err(error) = fs::rename(staged_agent, agent_path) {
        let _ = fs::rename(&rollback_agent, agent_path);
        let _ = fs::remove_file(staged_cli);
        return Err(error).context("activate downloaded infra-agent");
    }

    let reconnect = async {
        startup::restart()?;
        wait_for_online_reconnect(
            state_dir,
            previous_runtime.as_ref(),
            Duration::from_secs(20),
        )
        .await
    }
    .await;
    if let Err(error) = reconnect {
        let rollback = async {
            let _ = fs::remove_file(agent_path);
            fs::rename(&rollback_agent, agent_path)
                .context("restore previous infra-agent binary")?;
            startup::recover_after_failed_start()?;
            wait_for_online_reconnect(state_dir, None, Duration::from_secs(20)).await?;
            anyhow::Ok(())
        }
        .await;
        let _ = fs::remove_file(staged_cli);
        return match rollback {
            Ok(()) => Err(anyhow!(
                "new infra-agent failed its reconnect health check; previous agent was restored and is online: {error:#}"
            )),
            Err(rollback_error) => Err(anyhow!(
                "new infra-agent failed its reconnect health check and automatic rollback did not recover the agent: update error: {error:#}; rollback error: {rollback_error:#}"
            )),
        };
    }

    fs::rename(cli_path, &rollback_cli)
        .with_context(|| format!("stage rollback copy of {}", cli_path.display()))?;
    if let Err(error) = fs::rename(staged_cli, cli_path) {
        let _ = fs::rename(&rollback_cli, cli_path);
        let agent_rollback = async {
            let _ = fs::remove_file(agent_path);
            fs::rename(&rollback_agent, agent_path)
                .context("restore previous infra-agent after CLI activation failure")?;
            startup::recover_after_failed_start()?;
            wait_for_online_reconnect(state_dir, None, Duration::from_secs(20)).await?;
            anyhow::Ok(())
        }
        .await;
        return match agent_rollback {
            Ok(()) => Err(error).context(
                "activate downloaded infra CLI; previous agent was restored and is online",
            ),
            Err(rollback_error) => Err(anyhow!(
                "activate downloaded infra CLI failed and agent rollback did not recover connectivity: CLI error: {error}; rollback error: {rollback_error:#}"
            )),
        };
    }

    println!(
        "Infra updated from {base}; the new agent is online. Previous binaries are retained as infra.rollback and infra-agent.rollback."
    );
    #[cfg(target_os = "macos")]
    println!(
        "macOS may require Accessibility and Screen Recording to be re-enabled after replacing the agent binary; verify desktop permissions before relying on GUI control."
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(windows)]
async fn apply_windows_update(
    state_dir: &Path,
    base: &str,
    cli_path: &Path,
    agent_path: &Path,
    staged_cli: &Path,
    staged_agent: &Path,
) -> anyhow::Result<ExitCode> {
    let install_dir = cli_path
        .parent()
        .ok_or_else(|| anyhow!("infra CLI path has no parent"))?;
    let rollback_agent = install_dir.join("infra-agent.rollback.exe");
    let rollback_cli = install_dir.join("infra.rollback.exe");
    let _ = fs::remove_file(&rollback_agent);

    let previous_runtime = read_runtime(state_dir).ok().flatten();
    windows_stop_agent();
    if let Some(runtime) = &previous_runtime {
        wait_for_process_exit(runtime.pid, Duration::from_secs(10)).await?;
    }

    fs::rename(agent_path, &rollback_agent)
        .with_context(|| format!("stage rollback copy of {}", agent_path.display()))?;
    if let Err(error) = fs::rename(staged_agent, agent_path) {
        let _ = fs::rename(&rollback_agent, agent_path);
        let _ = fs::remove_file(staged_cli);
        let _ = windows_start_agent();
        return Err(error).context("activate downloaded infra-agent");
    }

    let reconnect = async {
        windows_start_agent()?;
        wait_for_online_reconnect(
            state_dir,
            previous_runtime.as_ref(),
            Duration::from_secs(25),
        )
        .await
    }
    .await;
    if let Err(error) = reconnect {
        windows_stop_agent();
        let _ = fs::remove_file(agent_path);
        let rollback_result = fs::rename(&rollback_agent, agent_path);
        if rollback_result.is_ok() {
            let _ = windows_start_agent();
            let _ = wait_for_online_reconnect(state_dir, None, Duration::from_secs(25)).await;
        }
        let _ = fs::remove_file(staged_cli);
        return Err(anyhow!(
            "new infra-agent failed its reconnect health check and rollback was attempted: {error:#}"
        ));
    }

    let helper = install_dir.join(format!(".infra-update-helper-{}.ps1", std::process::id()));
    write_windows_cli_update_helper(
        &helper,
        cli_path,
        staged_cli,
        &rollback_cli,
        std::process::id(),
    )?;
    Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&helper)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("start Windows CLI replacement helper")?;

    println!(
        "Infra agent updated from {base} and is online. The verified CLI replacement is staged and will be applied immediately after this command exits; the prior CLI is retained as infra.rollback.exe."
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(windows)]
fn windows_stop_agent() {
    let _ = Command::new("schtasks")
        .args(["/End", "/TN", "Infra MCP Agent"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(windows)]
fn windows_start_agent() -> anyhow::Result<()> {
    let status = Command::new("schtasks")
        .args(["/Run", "/TN", "Infra MCP Agent"])
        .status()?;
    if !status.success() {
        return Err(anyhow!("failed to start Infra MCP Agent scheduled task"));
    }
    Ok(())
}

#[cfg(windows)]
async fn wait_for_process_exit(pid: u32, timeout: Duration) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    while process_alive(pid) {
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!("agent PID {pid} did not stop for update"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

#[cfg(windows)]
fn ps_quote_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

#[cfg(windows)]
fn write_windows_cli_update_helper(
    helper: &Path,
    cli_path: &Path,
    staged_cli: &Path,
    rollback_cli: &Path,
    parent_pid: u32,
) -> anyhow::Result<()> {
    let cli = ps_quote_path(cli_path);
    let staged = ps_quote_path(staged_cli);
    let rollback = ps_quote_path(rollback_cli);
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$parentPid = {parent_pid}
$deadline = (Get-Date).AddSeconds(20)
while ((Get-Process -Id $parentPid -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) {{ Start-Sleep -Milliseconds 100 }}
if (Get-Process -Id $parentPid -ErrorAction SilentlyContinue) {{ exit 12 }}
$cli = '{cli}'
$staged = '{staged}'
$rollback = '{rollback}'
try {{
  if (Test-Path -LiteralPath $rollback) {{ Remove-Item -LiteralPath $rollback -Force }}
  Move-Item -LiteralPath $cli -Destination $rollback -Force
  Move-Item -LiteralPath $staged -Destination $cli -Force
  Remove-Item -LiteralPath $MyInvocation.MyCommand.Path -Force -ErrorAction SilentlyContinue
  exit 0
}} catch {{
  if (-not (Test-Path -LiteralPath $cli) -and (Test-Path -LiteralPath $rollback)) {{ Move-Item -LiteralPath $rollback -Destination $cli -Force }}
  exit 13
}}
"#
    );
    fs::write(helper, script).with_context(|| format!("write {}", helper.display()))
}

fn release_asset_descriptor() -> anyhow::Result<(String, &'static str)> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "macos",
        "windows" => "windows",
        other => return Err(anyhow!("unsupported update OS {other}")),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(anyhow!("unsupported update architecture {other}")),
    };
    #[cfg(windows)]
    let extension = ".exe";
    #[cfg(not(windows))]
    let extension = "";
    Ok((format!("{os}-{arch}"), extension))
}

async fn download_verified_release_asset(
    client: &reqwest::Client,
    base: &str,
    binary: &str,
    asset: &str,
    extension: &str,
    destination: &Path,
) -> anyhow::Result<()> {
    let name = format!("{binary}-{asset}{extension}");
    let bytes = client
        .get(format!("{base}/{name}"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let sidecar = client
        .get(format!("{base}/{name}.sha256"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let expected = sidecar
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("empty SHA-256 sidecar for {name}"))?
        .to_ascii_lowercase();
    if expected.len() != 64 || !expected.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(anyhow!("invalid SHA-256 sidecar for {name}"));
    }
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected {
        return Err(anyhow!(
            "SHA-256 mismatch for {name}: expected {expected}, got {actual}"
        ));
    }
    fs::write(destination, &bytes)
        .with_context(|| format!("write staged update {}", destination.display()))?;
    Ok(())
}

fn make_executable_like(path: &Path, reference: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let reference_mode = fs::metadata(reference)?.permissions().mode();
        fs::set_permissions(path, fs::Permissions::from_mode(reference_mode))?;
    }
    #[cfg(windows)]
    {
        let _ = (path, reference);
    }
    Ok(())
}

fn collect_status(state_dir: &Path) -> anyhow::Result<LocalStatus> {
    let config = read_config(state_dir)?;
    let runtime = read_runtime(state_dir)?;
    let alive = runtime
        .as_ref()
        .is_some_and(|runtime| process_alive(runtime.pid));
    Ok(LocalStatus {
        state_dir: state_dir.to_string_lossy().into_owned(),
        configured: config.is_some(),
        enrolled: identity_path(state_dir).exists(),
        process_alive: alive,
        startup: startup::status(),
        config,
        runtime,
    })
}

async fn doctor(state_dir: &Path) -> DoctorReport {
    let mut checks = Vec::new();
    checks.push(check(
        "state_dir",
        state_dir.is_dir(),
        if state_dir.is_dir() {
            format!("{} exists", state_dir.display())
        } else {
            format!("{} is missing", state_dir.display())
        },
        true,
    ));

    let config = match read_config(state_dir) {
        Ok(Some(config)) => {
            checks.push(check("config", true, "config.json is valid".into(), false));
            Some(config)
        }
        Ok(None) => {
            checks.push(check(
                "config",
                false,
                "config.json is missing".into(),
                false,
            ));
            None
        }
        Err(error) => {
            checks.push(check(
                "config",
                false,
                format!("config invalid: {error}"),
                false,
            ));
            None
        }
    };

    let enrolled = identity_path(state_dir).is_file();
    checks.push(check(
        "identity",
        enrolled,
        if enrolled {
            "device identity exists".into()
        } else {
            "device is not enrolled".into()
        },
        false,
    ));

    let runtime = read_runtime(state_dir).ok().flatten();
    if enrolled {
        checks.push(identity_storage_check(state_dir, runtime.as_ref()));
    }

    let alive = runtime
        .as_ref()
        .is_some_and(|runtime| process_alive(runtime.pid));
    checks.push(check(
        "agent_process",
        alive,
        match runtime {
            Some(runtime) if alive => {
                format!("agent PID {} is alive ({})", runtime.pid, runtime.state)
            }
            Some(runtime) => format!(
                "runtime references dead PID {} ({})",
                runtime.pid, runtime.state
            ),
            None => "runtime.json is missing".into(),
        },
        true,
    ));

    let startup = startup::status();
    checks.push(check(
        "startup",
        startup.installed && startup.active,
        startup.detail,
        true,
    ));

    if let Some(config) = config {
        match gateway_health_url(&config.gateway_url) {
            Ok(url) => {
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(3))
                    .build();
                match client {
                    Ok(client) => match client.get(url).send().await {
                        Ok(response) if response.status().is_success() => checks.push(check(
                            "gateway",
                            true,
                            "gateway health endpoint reachable".into(),
                            false,
                        )),
                        Ok(response) => checks.push(check(
                            "gateway",
                            false,
                            format!("gateway returned HTTP {}", response.status()),
                            false,
                        )),
                        Err(error) => checks.push(check(
                            "gateway",
                            false,
                            classify_network_error(&error),
                            false,
                        )),
                    },
                    Err(error) => checks.push(check(
                        "gateway",
                        false,
                        format!("HTTP client setup failed: {error}"),
                        false,
                    )),
                }
            }
            Err(error) => checks.push(check("gateway", false, error.to_string(), false)),
        }
    }

    DoctorReport {
        healthy: checks.iter().all(|check| check.status == "ok"),
        checks,
    }
}

fn repair(state_dir: &Path) -> anyhow::Result<()> {
    let config = read_config(state_dir)?
        .ok_or_else(|| anyhow!("cannot repair startup: config.json is missing"))?;
    if config.gateway_url.trim().is_empty() {
        return Err(anyhow!("cannot repair startup: gateway URL is empty"));
    }
    if !identity_path(state_dir).is_file() {
        return Err(anyhow!(
            "cannot repair startup: device identity is missing; re-enroll from the dashboard"
        ));
    }
    let agent = startup::sibling_agent_binary()?;
    if !agent.is_file() {
        return Err(anyhow!("agent binary not found at {}", agent.display()));
    }
    startup::install(&agent, state_dir)
}

fn remove_native_identity_credentials(state_dir: &Path) -> anyhow::Result<()> {
    for file_name in ["identity.json", "pending-enrollment.json"] {
        let path = state_dir.join(file_name);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode {} before credential cleanup", path.display()))?;
        if value["version"].as_u64() != Some(2)
            || value["storage"].as_str() != Some("native_keyring")
        {
            continue;
        }
        let key_id = value["key_id"]
            .as_str()
            .ok_or_else(|| anyhow!("{} is missing native credential key_id", path.display()))?;
        infra_platform::secret_store::delete_device_identity(key_id).with_context(|| {
            format!(
                "delete native device credential for {}; local state was preserved so cleanup can be retried",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn identity_storage_check(state_dir: &Path, runtime: Option<&RuntimeStatus>) -> DoctorCheck {
    let path = identity_path(state_dir);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return check(
                "credential_storage",
                false,
                format!("cannot read identity metadata: {error}"),
                false,
            );
        }
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return check(
                "credential_storage",
                false,
                format!("identity metadata is invalid: {error}"),
                false,
            );
        }
    };
    match value["version"].as_u64() {
        Some(1) => check(
            "credential_storage",
            false,
            "legacy identity stores the signing key in identity.json; restart/repair with the current agent to migrate".into(),
            true,
        ),
        Some(2) => match value["storage"].as_str() {
            Some("native_keyring") => {
                if value["key_id"].as_str().is_none() {
                    return check(
                        "credential_storage",
                        false,
                        "native credential metadata is missing key_id".into(),
                        false,
                    );
                }
                match runtime {
                    Some(runtime)
                        if runtime.state == "online"
                            && runtime.credential_storage.as_deref() == Some("native_keyring")
                            && runtime.credential_verified_unix_ms.is_some() =>
                    {
                        check(
                            "credential_storage",
                            true,
                            format!(
                                "native OS credential was loaded successfully by the online agent and used for signed authentication (verified at {})",
                                runtime.credential_verified_unix_ms.unwrap_or_default()
                            ),
                            false,
                        )
                    }
                    Some(runtime)
                        if runtime.credential_storage.as_deref() == Some("native_keyring")
                            && runtime.credential_verified_unix_ms.is_some() =>
                    {
                        check(
                            "credential_storage",
                            false,
                            "native OS credential was verified previously, but the agent is not currently online; reconnect the agent to verify it non-interactively".into(),
                            true,
                        )
                    }
                    _ => check(
                        "credential_storage",
                        false,
                        "native OS credential is configured but has not yet been verified by a successful signed agent connection".into(),
                        true,
                    ),
                }
            }
            Some("file_fallback") => check(
                "credential_storage",
                true,
                "native credential store unavailable; using the private state-file fallback".into(),
                false,
            ),
            Some(other) => check(
                "credential_storage",
                false,
                format!("unknown identity storage mode {other:?}"),
                false,
            ),
            None => check(
                "credential_storage",
                false,
                "identity metadata has no storage mode".into(),
                false,
            ),
        },
        Some(version) => check(
            "credential_storage",
            false,
            format!("unsupported identity metadata version {version}"),
            false,
        ),
        None => check(
            "credential_storage",
            false,
            "identity metadata has no version".into(),
            false,
        ),
    }
}

fn check(name: &'static str, ok: bool, detail: String, fixable: bool) -> DoctorCheck {
    DoctorCheck {
        name,
        status: if ok { "ok" } else { "problem" },
        detail,
        fixable: !ok && fixable,
    }
}

fn cleanup_persistent_workers(state_dir: &Path) -> usize {
    let mut candidates = HashSet::new();
    collect_worker_pids(
        &state_dir.join("sessions"),
        "session.json",
        "worker_pid",
        &mut candidates,
    );
    collect_worker_pids(
        &state_dir.join("jobs"),
        "job.json",
        "runner_pid",
        &mut candidates,
    );

    let system = System::new_all();
    let mut stopped = 0;
    for raw_pid in candidates {
        let pid = Pid::from_u32(raw_pid);
        let Some(process) = system.process(pid) else {
            continue;
        };
        if !is_infra_agent_process(process) {
            continue;
        }
        let mut descendants = Vec::new();
        collect_descendants(&system, pid, &mut descendants);
        for child_pid in descendants.into_iter().rev() {
            if let Some(child) = system.process(child_pid) {
                let _ = child.kill();
            }
        }
        let _ = process.kill();
        stopped += 1;
    }
    cleanup_platform_worker_registrations(state_dir);
    stopped
}

#[cfg(target_os = "macos")]
fn cleanup_platform_worker_registrations(state_dir: &Path) {
    for (directory, kind) in [("sessions", "pty"), ("jobs", "job")] {
        let Ok(entries) = fs::read_dir(state_dir.join(directory)) else {
            continue;
        };
        for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
            let id = entry.file_name().to_string_lossy().into_owned();
            let label = format!("io.phloraxx.infra.worker.{kind}.{id}");
            let _ = Command::new("launchctl")
                .args(["remove", &label])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn cleanup_platform_worker_registrations(_state_dir: &Path) {}

fn collect_worker_pids(root: &Path, meta_name: &str, key: &str, out: &mut HashSet<u32>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path().join(meta_name);
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        if let Some(pid) = value[key].as_u64().and_then(|pid| u32::try_from(pid).ok()) {
            out.insert(pid);
        }
    }
}

fn collect_descendants(system: &System, parent: Pid, out: &mut Vec<Pid>) {
    for (pid, process) in system.processes() {
        if process.parent() == Some(parent) {
            collect_descendants(system, *pid, out);
            out.push(*pid);
        }
    }
}

fn is_infra_agent_process(process: &sysinfo::Process) -> bool {
    let name = process.name().to_string_lossy().to_ascii_lowercase();
    let executable = process
        .exe()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name.contains("infra-agent") || executable.contains("infra-agent")
}

fn process_alive(pid: u32) -> bool {
    let system = System::new_all();
    system
        .process(Pid::from_u32(pid))
        .is_some_and(is_infra_agent_process)
}

fn gateway_health_url(gateway_ws_url: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(gateway_ws_url).context("parse configured gateway URL")?;
    let scheme = match url.scheme() {
        "ws" => "http",
        "wss" => "https",
        other => return Err(anyhow!("unsupported gateway URL scheme {other}")),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow!("cannot convert gateway URL to HTTP"))?;
    url.set_path("/healthz");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn classify_network_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return "gateway health check timed out".into();
    }
    if error.is_connect() {
        return format!("cannot connect to gateway: {error}");
    }
    format!("gateway health check failed: {error}")
}

fn take_option(args: &mut Vec<String>, name: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == name)?;
    args.remove(index);
    if index >= args.len() {
        return None;
    }
    Some(args.remove(index))
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(index) = args.iter().position(|arg| arg == name) {
        args.remove(index);
        true
    } else {
        false
    }
}

fn ensure_no_args(args: &[String]) -> anyhow::Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("unexpected arguments: {}", args.join(" ")))
    }
}

fn print_status(status: &LocalStatus) {
    println!("Infra status");
    println!("State dir: {}", status.state_dir);
    println!("Configured: {}", yes_no(status.configured));
    println!("Enrolled: {}", yes_no(status.enrolled));
    println!(
        "Agent process: {}",
        if status.process_alive {
            "running"
        } else {
            "not running"
        }
    );
    println!(
        "Startup: {} ({})",
        status.startup.mechanism, status.startup.detail
    );
    if let Some(runtime) = &status.runtime {
        println!("Connection state: {}", runtime.state);
        if let Some(device_id) = &runtime.device_id {
            println!("Device ID: {device_id}");
        }
        if let Some(error) = &runtime.last_error {
            println!("Last error: {error}");
        }
    }
}

fn print_doctor(report: &DoctorReport) {
    println!("Infra doctor\n");
    for item in &report.checks {
        println!(
            "{:<16} {:<7} {}{}",
            item.name,
            if item.status == "ok" { "OK" } else { "PROBLEM" },
            item.detail,
            if item.fixable { " [repairable]" } else { "" }
        );
    }
    println!(
        "\n{}",
        if report.healthy {
            "No problems found."
        } else {
            "Problems found. Run `infra repair` for repairable startup issues."
        }
    );
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn print_help() {
    println!(
        "Infra device CLI\n\n\
Usage:\n  infra status [--json] [--state-dir PATH]\n  infra doctor [--json] [--state-dir PATH]\n  infra install --gateway WS_URL [--code CODE] [--name NAME] [--agent PATH] [--no-startup] [--state-dir PATH]\n  infra repair [--state-dir PATH]\n  infra reconnect [--state-dir PATH]\n  infra update [--version TAG] [--repo OWNER/REPO] [--base-url URL] [--state-dir PATH]\n  infra uninstall [--remove-state] [--state-dir PATH]\n"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_url_tracks_tls() {
        assert_eq!(
            gateway_health_url("wss://example.invalid/agent")
                .unwrap()
                .as_str(),
            "https://example.invalid/healthz"
        );
        assert_eq!(
            gateway_health_url("ws://127.0.0.1:8788/agent")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:8788/healthz"
        );
    }

    #[test]
    fn argument_helpers_remove_values() {
        let mut args = vec!["--gateway".into(), "ws://x/agent".into(), "--json".into()];
        assert_eq!(
            take_option(&mut args, "--gateway").as_deref(),
            Some("ws://x/agent")
        );
        assert!(take_flag(&mut args, "--json"));
        assert!(args.is_empty());
    }

    #[test]
    fn reconnect_wait_requires_fresh_runtime_state() {
        let mut previous = RuntimeStatus::new("online");
        previous.pid = 42;
        previous.updated_unix_ms = 100;
        previous.connection_generation = Some(7);
        previous.last_connected_unix_ms = Some(90);

        let unchanged = previous.clone();
        assert!(!runtime_is_newer(Some(&previous), &unchanged));

        let mut refreshed = previous.clone();
        refreshed.updated_unix_ms = 101;
        assert!(runtime_is_newer(Some(&previous), &refreshed));

        let mut regenerated = previous.clone();
        regenerated.connection_generation = Some(8);
        assert!(runtime_is_newer(Some(&previous), &regenerated));
    }

    #[test]
    fn native_credential_doctor_uses_agent_runtime_proof() {
        let dir = tempfile::tempdir().unwrap();
        infra_local_state::ensure_state_dir(dir.path()).unwrap();
        fs::write(
            identity_path(dir.path()),
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": 2,
                "device_id": "test-device",
                "device_name": "Test Device",
                "key_id": "does-not-need-to-exist-for-doctor",
                "storage": "native_keyring"
            }))
            .unwrap(),
        )
        .unwrap();
        let mut runtime = RuntimeStatus::new("online");
        runtime.pid = std::process::id();
        runtime.credential_storage = Some("native_keyring".into());
        runtime.credential_verified_unix_ms = Some(123);
        infra_local_state::write_runtime(dir.path(), &runtime).unwrap();

        let result = identity_storage_check(dir.path(), Some(&runtime));
        assert_eq!(result.status, "ok");
        assert!(result.detail.contains("online agent"));
        assert!(result.detail.contains("signed authentication"));
    }
}
