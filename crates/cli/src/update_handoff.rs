use super::{activation, install, lifecycle_lock::LifecycleLocks};
use anyhow::{Context, Result};
#[cfg(not(windows))]
use anyhow::{anyhow, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::{fs, path::Path, process::ExitCode};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub id: String,
    pub phase: String,
    pub detail: String,
    pub owner_pid: u32,
    pub updated_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_verified: Option<bool>,
}
pub fn status(state: &Path) -> Option<UpdateStatus> {
    let mut status: UpdateStatus = fs::read(receipt_path(state, current_id(state)?))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())?;
    if status.phase == "pending" {
        let owner_alive = match sysinfo::System::new_all() {
            Ok(system) => system
                .process(sysinfo::Pid::from_u32(status.owner_pid))
                .is_some(),
            // If the process table cannot be read, keep the phase pending
            // and let the receipt timeout decide the outcome.
            Err(_) => true,
        };
        if !owner_alive
            || infra_local_state::now_ms().saturating_sub(status.updated_unix_ms) > 300_000
        {
            status.phase = "outcome_unknown".into();
            status.detail =
                "No completion receipt was recorded; inspect the pair and journal before retrying."
                    .into();
        }
    }
    Some(status)
}
fn current_id(state: &Path) -> Option<Uuid> {
    fs::read_to_string(state.join("update-current"))
        .ok()?
        .trim()
        .parse()
        .ok()
}
fn receipt_path(state: &Path, id: Uuid) -> std::path::PathBuf {
    state.join("updates").join(format!("{id}.json"))
}
fn record(state: &Path, id: Uuid, phase: &str, detail: &str) -> Result<()> {
    record_owner(state, id, phase, detail, std::process::id())
}
fn record_owner(state: &Path, id: Uuid, phase: &str, detail: &str, owner_pid: u32) -> Result<()> {
    fs::create_dir_all(state.join("updates"))?;
    let value = UpdateStatus {
        id: id.to_string(),
        phase: phase.into(),
        detail: detail.into(),
        owner_pid,
        updated_unix_ms: infra_local_state::now_ms(),
        context_verified: None,
    };
    let path = receipt_path(state, id);
    let temporary = state.join(format!(".update-status-{id}"));
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec(&value)?)?;
    file.sync_all()?;
    drop(file);
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(&path)?;
    }
    fs::rename(temporary, path)?;
    if phase == "pending" {
        fs::write(state.join("update-current"), id.to_string())?;
    }
    Ok(())
}

/// Probes a staged pair and pins both components to the signed release
/// version. Shared by the real handoff and `infra update --verify-only` so a
/// preflight cannot drift from what activation enforces.
pub async fn preflight(source: &Path, release_version: &str) -> Result<()> {
    let cli_version = install::probe(&source.join(activation::CLI), "infra").await?;
    let agent_version = install::probe(&source.join(activation::AGENT), "infra-agent").await?;
    super::release_verify::verify_binary_versions(release_version, &cli_version, &agent_version)
}

pub async fn apply(
    state: &Path,
    install_dir: &Path,
    cli: &Path,
    agent: &Path,
    release_version: &str,
    guard: LifecycleLocks,
) -> Result<ExitCode> {
    let id = Uuid::new_v4();
    let source = install_dir.join(format!(".infra-download-{id}"));
    fs::create_dir(&source)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700))?;
    }
    fs::copy(cli, source.join(activation::CLI))?;
    fs::copy(agent, source.join(activation::AGENT))?;
    let preflight = preflight(&source, release_version).await;
    if let Err(error) = preflight {
        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_file(cli);
        let _ = fs::remove_file(agent);
        return Err(error).context(
            "selected release does not support safe paired activation; installed files unchanged",
        );
    }
    // The private handoff directory now owns the checked candidate pair.
    // Windows cannot rely on the original updater surviving to clean downloads.
    let _ = fs::remove_file(cli);
    let _ = fs::remove_file(agent);
    let expected = serde_json::to_string(&activation::pair_hash(install_dir)?)?;
    fs::write(source.join("expected-pair.json"), &expected)?;
    record(
        state,
        id,
        "pending",
        "Verified download complete; pair activation has not completed.",
    )?;
    #[cfg(not(windows))]
    {
        drop(guard);
        let mut command = tokio::process::Command::new(source.join(activation::CLI));
        command
            .args(["activate", "--update", "--source-dir"])
            .arg(&source)
            .arg("--install-dir")
            .arg(install_dir)
            .arg("--state-dir")
            .arg(state)
            .arg("--expected-pair-file")
            .arg(source.join("expected-pair.json"));
        let result =
            infra_platform::command_runner::run_async(command, Duration::from_secs(180), 64 * 1024)
                .await;
        let success = matches!(&result,Ok(output) if output.exit_code==Some(0));
        record(
            state,
            id,
            if success {
                "complete"
            } else if result.is_err() {
                "outcome_unknown"
            } else {
                "failed"
            },
            if success {
                "Both binaries activated and fresh signed connectivity verified."
            } else {
                "Activation did not complete; inspect installer output and retained recovery files."
            },
        )?;
        if result.is_ok() {
            let _ = fs::remove_dir_all(&source);
        }
        match result {
            Ok(output) if success => {
                print!("{}", output.stdout);
                Ok(ExitCode::SUCCESS)
            }
            Ok(output) => bail!("paired update failed: {} {}", output.stdout, output.stderr),
            Err(error) => Err(anyhow!(
                "paired update interrupted; recovery files retained: {error:?}"
            )),
        }
    }
    #[cfg(windows)]
    {
        let quote = |path: &Path| path.to_string_lossy().replace('\'', "''");
        let script = source.join("activate-after-exit.ps1");
        let status = quote(&receipt_path(state, id));
        let log = quote(&state.join(format!("update-{id}.log")));
        let staged = quote(&source.join(activation::CLI));
        let src = quote(&source);
        let dest = quote(install_dir);
        let profile = quote(state);
        let parent = std::process::id();
        let text = format!(
            r#"param([Parameter(Mandatory=$true)][string]$ExpectedSid,[Parameter(Mandatory=$true)][ValidateSet(0,1)][int]$ExpectedAdmin)
$ErrorActionPreference='Stop'
$phase='outcome_unknown'
$context_verified=$false
try {{
  $deadline=(Get-Date).AddSeconds(20)
  while ((Get-Process -Id {parent} -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) {{ Start-Sleep -Milliseconds 100 }}
  if (Get-Process -Id {parent} -ErrorAction SilentlyContinue) {{ $phase='failed';throw 'Original updater did not exit; no activation attempted.' }}
  $identity=[Security.Principal.WindowsIdentity]::GetCurrent()
  $admin=([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  if ($identity.User.Value -ne $ExpectedSid -or [int]$admin -ne $ExpectedAdmin) {{ $phase='failed';throw 'Activation context differs from the caller; no activation attempted.' }}
  $context_verified=$true
  $arguments=@('activate','--update','--source-dir','"{src}"','--install-dir','"{dest}"','--state-dir','"{profile}"','--expected-pair-file','"{src}\expected-pair.json"')
  $child=Start-Process -FilePath '{staged}' -ArgumentList $arguments -NoNewWindow -Wait -PassThru -RedirectStandardOutput '{log}.out' -RedirectStandardError '{log}.err'
  [IO.File]::WriteAllText('{log}',([IO.File]::ReadAllText('{log}.out')+[IO.File]::ReadAllText('{log}.err')),([System.Text.UTF8Encoding]::new($false)))
  if ($child.ExitCode -eq 0) {{ $phase='complete' }} else {{ $phase='failed' }}
}} catch {{
  [IO.File]::AppendAllText('{log}',("Activation relay: "+$_.Exception.Message),([System.Text.UTF8Encoding]::new($false)))
  throw
}} finally {{
  $result=@{{id='{id}';phase=$phase;detail='Inspect the update log if activation failed; recovery files are retained.';owner_pid=$PID;updated_unix_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();context_verified=$context_verified}} | ConvertTo-Json -Compress
  [IO.File]::WriteAllText('{status}',$result,([System.Text.UTF8Encoding]::new($false)))
  Remove-Item -LiteralPath '{src}' -Recurse -Force -ErrorAction SilentlyContinue
}}
"#
        );
        // Windows PowerShell 5.1 needs the BOM to decode Unicode paths in source.
        fs::write(&script, format!("\u{feff}{text}"))?;
        // Local Win32_Process.Create is brokered outside the caller's Job
        // Object. ShellExecute/null stdio alone did not survive MCP exec cleanup.
        // The relay checks SID and effective administrator status before acting.
        let launcher_script = format!(
            r#"$ErrorActionPreference='Stop'
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$admin=[int](([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))
$exe=Join-Path $PSHOME 'powershell.exe'
$line='"'+$exe+'" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{script}" -ExpectedSid '+$identity.User.Value+' -ExpectedAdmin '+$admin
$startup=New-CimInstance -ClassName Win32_ProcessStartup -ClientOnly -Property @{{ShowWindow=[uint16]0}}
$created=Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{{CommandLine=$line;CurrentDirectory='{src}';ProcessStartupInformation=$startup}}
if ($created.ReturnValue -ne 0 -or $created.ProcessId -le 0) {{ throw 'Local activation process creation failed; no reliable relay was returned.' }}
$created.ProcessId
"#,
            script = quote(&script),
        );
        let mut launcher = tokio::process::Command::new("powershell.exe");
        launcher.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &launcher_script,
        ]);
        let launched =
            infra_platform::command_runner::run_async(launcher, Duration::from_secs(15), 4096)
                .await;
        let owner_pid = match launched {
            Ok(output) if output.exit_code == Some(0) && !output.truncated => output
                .stdout
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|pid| *pid > 0),
            _ => None,
        };
        let Some(owner_pid) = owner_pid else {
            record(
                state,
                id,
                "outcome_unknown",
                "No reliable relay PID was returned; inspect the receipt and pair before retrying.",
            )?;
            return Err(anyhow::anyhow!(
                "activation handoff outcome unknown; recovery files retained"
            ));
        };
        record_owner(
            state,
            id,
            "pending",
            "Waiting for native pair activation; not yet complete.",
            owner_pid,
        )?;
        drop(guard);
        println!(
            "Update {id} is pending. This command must exit before Windows can replace the CLI. Use infra status to inspect complete/failed; success is not yet confirmed."
        );
        Ok(ExitCode::SUCCESS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_completion_cannot_replace_a_newer_update_and_lost_owner_is_uncertain() {
        let dir = tempfile::tempdir().unwrap();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        record(dir.path(), first, "pending", "first").unwrap();
        record(dir.path(), second, "pending", "second").unwrap();
        record(dir.path(), first, "complete", "first complete").unwrap();
        let latest = status(dir.path()).unwrap();
        assert_eq!(latest.id, second.to_string());
        assert_eq!(latest.phase, "pending");
        record_owner(dir.path(), second, "pending", "lost owner", u32::MAX).unwrap();
        assert_eq!(status(dir.path()).unwrap().phase, "outcome_unknown");
    }
}
