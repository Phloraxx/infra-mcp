mod activation;
mod doctor;
mod install;
mod lifecycle_lock;
mod readiness;
mod release_verify;
mod startup;
mod status;
mod update;
mod update_handoff;

use anyhow::{Context, anyhow};
use doctor::*;
use infra_local_state::{
    LocalConfig, RuntimeStatus, default_state_dir, identity_path, read_config, read_runtime,
    write_config,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use status::*;
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
use update::*;
use url::Url;

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

    if command == "--build-info" {
        ensure_no_args(&args)?;
        println!(
            "{}",
            serde_json::json!({"component":"infra","activation_protocol":1,"version":env!("CARGO_PKG_VERSION")})
        );
        return Ok(ExitCode::SUCCESS);
    }
    let operation_lock = if matches!(
        command.as_str(),
        "setup" | "install" | "repair" | "reconnect" | "update" | "uninstall"
    ) {
        Some(lifecycle_lock::LifecycleLocks::acquire(
            &lifecycle_lock::current_install_dir()?,
            &state_dir,
        )?)
    } else {
        None
    };
    if operation_lock.is_some() && activation::pending(&lifecycle_lock::current_install_dir()?) {
        return Err(anyhow!(
            "activation_recovery_required: rerun the verified installer to recover the interrupted pair"
        ));
    }

    match command.as_str() {
        "activate" => install::activate(&state_dir, args).await,
        "status" => {
            let verify = take_flag(&mut args, "--verify");
            let timeout_requested = args.iter().any(|value| value == "--timeout-ms");
            let timeout = take_option(&mut args, "--timeout-ms");
            if timeout_requested && timeout.is_none() {
                return Err(anyhow!("--timeout-ms requires a value"));
            }
            if timeout.is_some() && !verify {
                return Err(anyhow!("--timeout-ms requires status --verify"));
            }
            let timeout = timeout
                .map(|value| value.parse::<u64>())
                .transpose()
                .context("--timeout-ms must be an integer")?
                .unwrap_or(15_000);
            if !(1..=60_000).contains(&timeout) {
                return Err(anyhow!("--timeout-ms must be between 1 and 60000"));
            }
            ensure_no_args(&args)?;
            let verification_failed = verify
                && wait_for_online_reconnect(&state_dir, None, Duration::from_millis(timeout))
                    .await
                    .is_err();
            // Verification uses local receipts/process state only. Avoid
            // unbounded launchctl/systemctl/PowerShell inspection in this mode.
            let mut status = collect_status(&state_dir, !verify)?;
            if verification_failed {
                status.connection.verified = false;
                status.connection.code = "connection_verification_failed";
                status.connection.detail = "Fresh signed status verification did not complete. Run doctor and check connectivity and gateway/agent compatibility.";
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                print_status(&status);
            }
            Ok(if verify && !status.connection.verified {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
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
        "setup" => setup_command(&state_dir, args).await,
        "repair" => {
            ensure_no_args(&args)?;
            repair(&state_dir)?;
            let runtime =
                wait_for_online_reconnect(&state_dir, None, Duration::from_secs(15)).await?;
            println!(
                "Infra startup registration repaired; fresh signed status roundtrip verified{}.",
                runtime
                    .connection_generation
                    .map(|generation| format!(" (generation {generation})"))
                    .unwrap_or_default()
            );
            Ok(ExitCode::SUCCESS)
        }
        "reconnect" => {
            ensure_no_args(&args)?;
            let _registration =
                startup::owned_registration(&startup::sibling_agent_binary()?, &state_dir)?;
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
        "update" => {
            update_command(
                &state_dir,
                args,
                operation_lock.expect("update lock acquired"),
            )
            .await
        }
        "uninstall" => {
            let remove_state = args.iter().any(|arg| arg == "--remove-state");
            let unknown = args
                .iter()
                .filter(|arg| arg.as_str() != "--remove-state")
                .cloned()
                .collect::<Vec<_>>();
            ensure_no_args(&unknown)?;
            let _registration =
                startup::owned_registration(&startup::sibling_agent_binary()?, &state_dir)?;
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

// The native setup command is the single lifecycle entry point. The thin
// platform scripts will eventually download/verify binaries and invoke it.
async fn setup_command(state_dir: &Path, mut args: Vec<String>) -> anyhow::Result<ExitCode> {
    let supplied_gateway =
        take_option(&mut args, "--gateway").or_else(|| std::env::var("INFRA_GATEWAY_URL").ok());
    let agent = take_option(&mut args, "--agent")
        .map(PathBuf::from)
        .unwrap_or(startup::sibling_agent_binary()?);
    let no_startup = take_flag(&mut args, "--no-startup");
    ensure_no_args(&args)?;
    if !agent.is_file() {
        return Err(anyhow!(
            "Infra device agent was not installed at {}; download both verified binaries first",
            agent.display()
        ));
    }
    let previous = read_config(state_dir)?;
    let gateway = match (supplied_gateway, previous.as_ref()) {
        (Some(next), Some(config))
            if next != config.gateway_url && identity_path(state_dir).exists() =>
        {
            return Err(anyhow!(
                "This computer is already paired to a different gateway. Setup will not silently move its identity. Use the explicit gateway migration flow."
            ));
        }
        (Some(url), _) => url,
        (None, Some(config)) => config.gateway_url.clone(),
        (None, None) => {
            return Err(anyhow!(
                "Infra control endpoint is not configured. Start setup from the dashboard's verified installer or supply --gateway for development."
            ));
        }
    };
    // Repair must preserve an intentional name override on older installs.
    // Brand-new installs leave it unset so the agent detects its OS hostname.
    let previous_name = previous
        .as_ref()
        .and_then(|config| config.device_name.clone());
    let next_config = LocalConfig::new(gateway.clone(), previous_name)?;
    if !identity_path(state_dir).is_file() {
        println!("Infra setup — pairing this computer using its system name.");
        let status = Command::new(&agent)
            .args(["--pair-only", "--state-dir"])
            .arg(state_dir)
            .env("INFRA_GATEWAY_URL", &gateway)
            .status()
            .with_context(|| "start signed pairing agent")?;
        if !status.success() {
            return Err(anyhow!(
                "Pairing was not completed. Your pending device identity is safe; run the same Infra setup command to retry."
            ));
        }
    }
    if previous.as_ref() != Some(&next_config) {
        // Preserve the old configuration until pairing succeeds. New setup does
        // not replace a healthy existing gateway or any signing key.
        write_config(state_dir, &next_config)?;
    }
    if no_startup {
        println!("Infra paired and configured; automatic startup skipped.");
        return Ok(ExitCode::SUCCESS);
    }
    startup::install(&agent, state_dir)
        .context("Infra paired, but automatic startup needs repair; run infra doctor")?;
    // An existing healthy Windows task need not create a new generation, but
    // its next signed status roundtrip must be freshly confirmed by the gateway.
    let runtime = wait_for_online_reconnect(
        state_dir,
        None,
        Duration::from_secs(35),
    ).await.context("Infra is installed and paired, but the agent did not confirm an online connection. Run infra doctor.")?;
    println!(
        "Infra connected: {} (signed connection generation {}).",
        runtime.device_name.as_deref().unwrap_or("this computer"),
        runtime.connection_generation.unwrap_or_default(),
    );
    Ok(ExitCode::SUCCESS)
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
    if let Some(previous) = read_config(state_dir)?
        && identity_path(state_dir).is_file()
        && previous.gateway_url != config.gateway_url
    {
        return Err(anyhow!(
            "already paired to another gateway; explicit migration is required"
        ));
    }

    if !identity_path(state_dir).exists() {
        let code = code.ok_or_else(|| {
            anyhow!("device is not enrolled; provide the one-time dashboard code with --code")
        })?;
        let mut enrollment = Command::new(&agent);
        enrollment
            .arg("--state-dir")
            .arg(state_dir)
            .arg("--enroll-only")
            .env("INFRA_GATEWAY_URL", &config.gateway_url)
            .env("INFRA_ENROLLMENT_CODE", code);
        if let Some(name) = &config.device_name {
            enrollment.env("INFRA_DEVICE_NAME", name);
        }
        let status = enrollment
            .status()
            .with_context(|| format!("run {} enrollment", agent.display()))?;
        if !status.success() {
            return Err(anyhow!("agent enrollment failed with {status}"));
        }
    }

    write_config(state_dir, &config)?;
    if no_startup {
        println!("Infra configured and enrolled. Startup registration skipped.");
        return Ok(ExitCode::SUCCESS);
    }
    startup::install(&agent, state_dir)?;
    wait_for_online_reconnect(state_dir, None, Duration::from_secs(35)).await?;
    println!(
        "Infra installed; fresh signed status roundtrip verified. Automatic startup is configured."
    );
    Ok(ExitCode::SUCCESS)
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

fn print_help() {
    println!(
        "Infra device CLI\n\n\
Usage:\n  infra status [--verify [--timeout-ms 15000]] [--json] [--state-dir PATH]\n  infra doctor [--json] [--state-dir PATH]\n  infra activate --source-dir PATH --install-dir PATH [--state-dir PATH] [setup options]\n  infra setup [--gateway WS_URL] [--agent PATH] [--no-startup] [--state-dir PATH]\n  infra install --gateway WS_URL [--code CODE] [--name NAME] [--agent PATH] [--no-startup] [--state-dir PATH]\n  infra repair [--state-dir PATH]\n  infra reconnect [--state-dir PATH]\n  infra update [--version TAG] [--repo OWNER/REPO] [--base-url URL] [--trusted-update-key BASE64] [--verify-only] [--state-dir PATH]\n  infra uninstall [--remove-state] [--state-dir PATH]\n"
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
        assert!(!runtime_is_newer(Some(&previous), &refreshed));

        let mut regenerated = previous.clone();
        regenerated.connection_generation = Some(8);
        assert!(runtime_is_newer(Some(&previous), &regenerated));
    }

    #[tokio::test]
    async fn readiness_wait_rejects_cached_online_then_accepts_a_fresh_same_generation_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let config = LocalConfig::new("ws://127.0.0.1:12345/agent".into(), None).unwrap();
        write_config(dir.path(), &config).unwrap();
        let runtime: RuntimeStatus = serde_json::from_value(serde_json::json!({
            "version":1,"pid":std::process::id(),"state":"online","device_id":"fixture",
            "connection_generation":7,"gateway_epoch":"11111111-1111-4111-8111-111111111111",
            "agent_instance_id":"22222222-2222-4222-8222-222222222222","updated_unix_ms":1
        }))
        .unwrap();
        infra_local_state::write_runtime(dir.path(), &runtime).unwrap();
        let mut proof = infra_local_state::ConnectionProof {
            version: 1,
            pid: runtime.pid,
            device_id: "fixture".into(),
            gateway_fingerprint: infra_local_state::gateway_fingerprint(&config.gateway_url),
            gateway_epoch: runtime.gateway_epoch.unwrap(),
            agent_instance_id: runtime.agent_instance_id.unwrap(),
            connection_generation: 7,
            probe_started_unix_ms: infra_local_state::now_ms().saturating_sub(100),
            verified_unix_ms: infra_local_state::now_ms().saturating_sub(100),
        };
        infra_local_state::write_connection_proof(dir.path(), &proof).unwrap();
        assert!(
            wait_for_verified_runtime(dir.path(), None, Duration::from_millis(80), |_| true)
                .await
                .is_err()
        );
        let path = dir.path().to_owned();
        // A delayed confirmation of a pre-check probe cannot certify a new check.
        let delayed = proof.clone();
        let delayed_path = path.clone();
        let delayed_writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let mut delayed = delayed;
            delayed.verified_unix_ms = infra_local_state::now_ms();
            infra_local_state::write_connection_proof(&delayed_path, &delayed).unwrap();
        });
        assert!(
            wait_for_verified_runtime(dir.path(), None, Duration::from_millis(80), |_| true)
                .await
                .is_err()
        );
        delayed_writer.await.unwrap();
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            proof.verified_unix_ms = infra_local_state::now_ms();
            proof.probe_started_unix_ms = proof.verified_unix_ms;
            infra_local_state::write_connection_proof(&path, &proof).unwrap();
        });
        let current = wait_for_verified_runtime(dir.path(), None, Duration::from_secs(2), |_| true)
            .await
            .unwrap();
        assert_eq!(current.pid, runtime.pid);
        assert_eq!(current.connection_generation, Some(7));
        writer.await.unwrap();
        assert!(
            wait_for_verified_runtime(
                dir.path(),
                Some(&current),
                Duration::from_millis(80),
                |_| true
            )
            .await
            .is_err()
        );
        let snapshot = collect_status(dir.path(), false).unwrap();
        assert!(snapshot.startup.is_none());
        assert!(!snapshot.connection.verified); // The real observer rejects the CLI test process as an agent.
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
