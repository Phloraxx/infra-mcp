//! Native coordinator shared by the download adapters.
use super::{
    activation::{self, PairTransaction},
    lifecycle_lock::LifecycleLocks,
    startup,
};
use anyhow::{Context, Result, anyhow, bail};
use infra_platform::command_runner::{self, CommandError};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

pub async fn probe(binary: &Path, component: &str) -> Result<String> {
    let mut command = tokio::process::Command::new(binary);
    command.arg("--build-info");
    let output = command_runner::run_async(command, Duration::from_secs(5), 4096)
        .await
        .map_err(|error| match error {
            CommandError::Failed(message) => anyhow!(message),
            CommandError::DeadlineExceeded(_) => anyhow!("binary preflight timed out"),
        })?;
    if output.exit_code != Some(0) || output.truncated {
        bail!("candidate build-info failed for {component}");
    }
    let info: serde_json::Value = serde_json::from_str(&output.stdout)
        .context("candidate must support native pair activation")?;
    if info["component"] != component
        || info["activation_protocol"] != 1
        || info["version"].as_str().is_none_or(str::is_empty)
    {
        bail!("candidate binary pair is incompatible with this installer");
    }
    Ok(info["version"].as_str().unwrap().to_owned())
}

pub async fn activate(state: &Path, mut args: Vec<String>) -> Result<ExitCode> {
    let source = super::take_option(&mut args, "--source-dir")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("activate requires --source-dir"))?;
    let install = super::take_option(&mut args, "--install-dir")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("activate requires --install-dir"))?;
    let update = super::take_flag(&mut args, "--update");
    let expected = super::take_option(&mut args, "--expected-pair-file")
        .map(|value| -> Result<activation::PairHash> {
            Ok(serde_json::from_slice(&std::fs::read(value)?)?)
        })
        .transpose()?;
    let no_startup = args.iter().any(|arg| arg == "--no-startup");
    if update {
        super::ensure_no_args(&args)?;
        if infra_local_state::read_config(state)?.is_none()
            || !infra_local_state::identity_path(state).is_file()
        {
            bail!("cannot activate update: enrolled profile is missing");
        }
    }
    let _locks = LifecycleLocks::acquire(&install, state)?;
    let agent = install.join(activation::AGENT);
    if activation::pending(&install) {
        activation::validate_recovery_profile(&install, state)?;
        if super::process_alive(infra_local_state::read_runtime(state)?.map_or(0, |r| r.pid)) {
            stop_owned_if_running(&agent, state).await?;
        }
        PairTransaction::recover(&install, state)?;
    }
    if let Some(expected) = expected
        && activation::pair_hash(&install)? != expected
    {
        bail!("installation_changed: another operation changed this pair; update was not applied");
    }
    if activation::hash(&std::env::current_exe()?)?
        != activation::hash(&source.join(activation::CLI))?
    {
        bail!("activate must run the staged candidate CLI");
    }
    if probe(&source.join(activation::AGENT), "infra-agent").await? != env!("CARGO_PKG_VERSION") {
        bail!("candidate CLI/agent package versions do not match");
    }
    if !no_startup {
        startup::ensure_owned_registration(&agent, state)?;
    }
    let transaction = PairTransaction::prepare(&install, state, &source)?;
    if transaction.changed()
        && !no_startup
        && let Err(error) = stop_owned_if_running(&agent, state).await
    {
        transaction.rollback()?;
        return Err(error);
    }
    let result = async {
        transaction.activate()?;
        if update {
            startup::install(&agent, state)?;
            super::wait_for_online_reconnect(state, None, Duration::from_secs(35)).await?;
            Ok(ExitCode::SUCCESS)
        } else {
            let legacy = args.iter().any(|arg| arg == "--code");
            args.extend(["--agent".into(), agent.to_string_lossy().into_owned()]);
            if legacy {
                super::install_command(state, args).await
            } else {
                super::setup_command(state, args).await
            }
        }
    }
    .await;
    match result {
        Ok(code) => {
            transaction.commit()?;
            println!("Infra binary pair activated; native setup completed.");
            Ok(code)
        }
        Err(error) if !transaction.has_previous() && transaction.fully_active() => {
            transaction.commit()?;
            Err(error)
                .context("pair installed; setup is incomplete and pending identity is retained")
        }
        Err(error) => {
            if !no_startup {
                stop_owned_if_running(&agent, state).await?;
            }
            transaction
                .rollback()
                .with_context(|| format!("activation failed: {error:#}"))?;
            if !no_startup && transaction.has_previous() && agent.is_file() {
                #[cfg(target_os = "linux")]
                startup::reset_failed()?;
                startup::install(&agent, state)
                    .context("previous pair restored, startup recovery failed")?;
                super::wait_for_online_reconnect(state, None, Duration::from_secs(35))
                    .await
                    .context("previous pair restored, fresh recovery not verified")?;
            }
            Err(error).context("activation failed; previous pair restored and identity retained")
        }
    }
}
async fn stop_owned_if_running(agent: &Path, state: &Path) -> Result<()> {
    let Some(runtime) = infra_local_state::read_runtime(state)? else {
        return Ok(());
    };
    if !super::process_alive(runtime.pid) {
        return Ok(());
    }
    let _registration = startup::owned_registration(agent, state)?;
    if !startup::owns(agent, state)? {
        bail!(
            "startup_owner_mismatch: refusing to stop startup belonging to another install/profile"
        );
    }
    startup::stop()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while super::process_alive(runtime.pid) {
        if tokio::time::Instant::now() >= deadline {
            bail!("agent_stop_timeout: preserve recovery files");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
