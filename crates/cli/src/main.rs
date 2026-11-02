mod activation;
mod install;
mod lifecycle_lock;
mod readiness;
mod release_verify;
mod startup;
mod update_handoff;

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
    connection: readiness::ConnectionStatus,
    startup: Option<startup::StartupStatus>,
    config: Option<LocalConfig>,
    runtime: Option<RuntimeStatus>,
    update_status: Option<update_handoff::UpdateStatus>,
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

async fn wait_for_online_reconnect(
    state_dir: &Path,
    previous: Option<&RuntimeStatus>,
    timeout: Duration,
) -> anyhow::Result<RuntimeStatus> {
    wait_for_verified_runtime(state_dir, previous, timeout, process_alive).await
}

async fn wait_for_verified_runtime(
    state_dir: &Path,
    previous: Option<&RuntimeStatus>,
    timeout: Duration,
    agent_alive: fn(u32) -> bool,
) -> anyhow::Result<RuntimeStatus> {
    let started_unix_ms = infra_local_state::now_ms();
    let config = read_config(state_dir)?.ok_or_else(|| anyhow!("connection_unverified: config.json is missing; complete setup before connection verification"))?;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        if let Ok(Some(runtime)) = read_runtime(state_dir)
            && runtime_is_newer(previous, &runtime)
        {
            let connection = readiness::inspect(
                state_dir,
                Some(&config),
                Some(&runtime),
                agent_alive(runtime.pid),
                infra_local_state::now_ms(),
            );
            if tokio::time::Instant::now() < deadline
                && connection.verified
                && connection
                    .probe_started_unix_ms
                    .is_some_and(|time| time > started_unix_ms)
            {
                return Ok(runtime);
            }
        }

        tokio::time::sleep_until(
            deadline.min(tokio::time::Instant::now() + Duration::from_millis(250)),
        )
        .await;
    }
    Err(anyhow!(
        "connection_unverified: no fresh signed status roundtrip was confirmed before the deadline; run `infra status` or `infra doctor`, and update both gateway and agent if verification is unsupported"
    ))
}

fn runtime_is_newer(previous: Option<&RuntimeStatus>, current: &RuntimeStatus) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    current.pid != previous.pid
        || current.gateway_epoch != previous.gateway_epoch
        || current.agent_instance_id != previous.agent_instance_id
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

async fn update_command(
    state_dir: &Path,
    mut args: Vec<String>,
    guard: lifecycle_lock::LifecycleLocks,
) -> anyhow::Result<ExitCode> {
    let repo = take_option(&mut args, "--repo").unwrap_or_else(|| "Phloraxx/infra-mcp".into());
    let version = take_option(&mut args, "--version").unwrap_or_else(|| "latest".into());
    let base_url = take_option(&mut args, "--base-url");
    let explicitly_trusted = take_option(&mut args, "--trusted-update-key")
        .or_else(|| std::env::var("INFRA_RELEASE_PUBLIC_KEY_B64").ok())
        .filter(|key| !key.trim().is_empty());
    let pinned = option_env!("INFRA_RELEASE_PUBLIC_KEY_B64").filter(|key| !key.trim().is_empty());
    if let (Some(pinned), Some(supplied)) = (pinned, &explicitly_trusted)
        && pinned != supplied.trim()
    {
        return Err(anyhow!(
            "this Infra binary has a pinned update key; a different key cannot override it"
        ));
    }
    let public_key = pinned.map(str::to_owned)
        .or(explicitly_trusted)
        .ok_or_else(|| anyhow!(
            "updates require an Ed25519 release trust key (set --trusted-update-key or compile with INFRA_RELEASE_PUBLIC_KEY_B64); refusing unsigned update"
        ))?;
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
    let using_default_release_url = base_url.is_none();
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

    // Signature is checked before any staged executable is written.
    let manifest_response = client
        .get(format!("{base}/infra-release.json"))
        .send()
        .await?;
    if manifest_response.status() == reqwest::StatusCode::NOT_FOUND
        && version == "latest"
        && using_default_release_url
    {
        return Err(anyhow!(
            "No stable Infra release is published. GitHub /releases/latest excludes prereleases; this is not an update or network repair failure. Use an explicitly tested signed prerelease tag such as --version v0.1.0-rc.1 for a manual update; unattended updates should remain disabled until a stable release passes Windows rollback acceptance."
        ));
    }
    let manifest_bytes = manifest_response.error_for_status()?.bytes().await?;
    let manifest_signature = client
        .get(format!("{base}/infra-release.json.sig"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let manifest =
        release_verify::verify_manifest(&manifest_bytes, &manifest_signature, &public_key)?;
    if version != "latest" && manifest.version != version {
        return Err(anyhow!(
            "signed release version {} does not match requested {version}",
            manifest.version
        ));
    }

    let staged_cli = install_dir.join(format!(".infra.update.{}{}", std::process::id(), extension));
    let staged_agent = install_dir.join(format!(
        ".infra-agent.update.{}{}",
        std::process::id(),
        extension
    ));
    download_verified_release_asset(
        &client,
        &base,
        "infra",
        &asset,
        extension,
        &staged_cli,
        &manifest,
    )
    .await?;
    if let Err(error) = download_verified_release_asset(
        &client,
        &base,
        "infra-agent",
        &asset,
        extension,
        &staged_agent,
        &manifest,
    )
    .await
    {
        let _ = fs::remove_file(&staged_cli);
        return Err(error);
    }
    make_executable_like(&staged_cli, &cli_path)?;
    make_executable_like(&staged_agent, &agent_path)?;

    update_handoff::apply(state_dir, install_dir, &staged_cli, &staged_agent, guard).await
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
    manifest: &release_verify::ReleaseManifest,
) -> anyhow::Result<()> {
    let name = format!("{binary}-{asset}{extension}");
    let bytes = client
        .get(format!("{base}/{name}"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let expected = manifest
        .assets
        .get(&name)
        .ok_or_else(|| anyhow!("asset {name} is missing from the signed release manifest"))?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != *expected {
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

fn collect_status(state_dir: &Path, check_startup: bool) -> anyhow::Result<LocalStatus> {
    let config = read_config(state_dir)?;
    let runtime = read_runtime(state_dir)?;
    let alive = runtime
        .as_ref()
        .is_some_and(|runtime| process_alive(runtime.pid));
    let connection = readiness::inspect(
        state_dir,
        config.as_ref(),
        runtime.as_ref(),
        alive,
        infra_local_state::now_ms(),
    );
    Ok(LocalStatus {
        state_dir: state_dir.to_string_lossy().into_owned(),
        configured: config.is_some(),
        enrolled: identity_path(state_dir).exists(),
        update_status: update_handoff::status(state_dir),
        process_alive: alive,
        connection,
        startup: check_startup.then(startup::status),
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
    let connection = readiness::inspect(
        state_dir,
        config.as_ref(),
        runtime.as_ref(),
        alive,
        infra_local_state::now_ms(),
    );
    checks.push(check(
        "connection",
        connection.verified,
        format!("{}: {}", connection.code, connection.detail),
        false,
    ));
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
        startup.installed && startup.active && startup.reliable,
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
    if let Some(update) = &status.update_status {
        println!("Update {}: {} ({})", update.id, update.phase, update.detail);
    }
    println!("State dir: {}", status.state_dir);
    println!("Configured: {}", yes_no(status.configured));
    println!("Enrolled: {}", yes_no(status.enrolled));
    println!(
        "Connection verification: {} ({})",
        status.connection.code, status.connection.detail
    );
    println!(
        "Agent process: {}",
        if status.process_alive {
            "running"
        } else {
            "not running"
        }
    );
    if let Some(startup) = &status.startup {
        println!("Startup: {} ({})", startup.mechanism, startup.detail);
    } else {
        println!(
            "Startup: not inspected by connection verification; use infra status for startup details."
        );
    }
    if let Some(runtime) = &status.runtime {
        println!("Recorded agent state: {}", runtime.state);
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
Usage:\n  infra status [--verify [--timeout-ms 15000]] [--json] [--state-dir PATH]\n  infra doctor [--json] [--state-dir PATH]\n  infra activate --source-dir PATH --install-dir PATH [--state-dir PATH] [setup options]\n  infra setup [--gateway WS_URL] [--agent PATH] [--no-startup] [--state-dir PATH]\n  infra install --gateway WS_URL [--code CODE] [--name NAME] [--agent PATH] [--no-startup] [--state-dir PATH]\n  infra repair [--state-dir PATH]\n  infra reconnect [--state-dir PATH]\n  infra update [--version TAG] [--repo OWNER/REPO] [--base-url URL] [--trusted-update-key BASE64] [--state-dir PATH]\n  infra uninstall [--remove-state] [--state-dir PATH]\n"
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
