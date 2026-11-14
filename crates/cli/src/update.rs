use super::*;

pub(crate) async fn wait_for_online_reconnect(
    state_dir: &Path,
    previous: Option<&RuntimeStatus>,
    timeout: Duration,
) -> anyhow::Result<RuntimeStatus> {
    wait_for_verified_runtime(state_dir, previous, timeout, process_alive).await
}
pub(crate) async fn wait_for_verified_runtime(
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
pub(crate) fn runtime_is_newer(previous: Option<&RuntimeStatus>, current: &RuntimeStatus) -> bool {
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
pub(crate) async fn update_command(
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
            "No stable Infra release is published. GitHub /releases/latest excludes prereleases; this is not an update or network repair failure. Use --version <compatible-tag> only for a tested signed prerelease that supports native activation and is not older than this CLI; no update was applied."
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
    let release_version = release_verify::authorize_update_version(
        &manifest.version,
        env!("CARGO_PKG_VERSION"),
        &version,
    )?;
    let floor = release_verify::read_release_floor(state_dir)?;
    let release_index = release_verify::authorize_release_index(manifest.release_index, floor)?;

    // Stage under canonical names in a private directory so the preflight
    // probes exactly the pair activation would install.
    let staging = install_dir.join(format!(".infra-stage-{}", std::process::id()));
    fs::create_dir_all(&staging)?;
    let staged_cli = staging.join(format!("infra{extension}"));
    let staged_agent = staging.join(format!("infra-agent{extension}"));
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
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    make_executable_like(&staged_cli, &cli_path)?;
    make_executable_like(&staged_agent, &agent_path)?;

    if let Err(error) = update_handoff::preflight(&staging, &release_version).await {
        let _ = fs::remove_dir_all(&staging);
        return Err(error).context(
            "selected release does not support safe paired activation; installed files unchanged",
        );
    }
    // The floor records the highest release this device has verified and begun
    // installing. It is monotonic, so a failed activation can never lower it.
    release_verify::record_release_floor(state_dir, release_index, &release_version)?;

    let result = update_handoff::apply(
        state_dir,
        install_dir,
        &staged_cli,
        &staged_agent,
        &release_version,
        guard,
    )
    .await;
    let _ = fs::remove_dir_all(&staging);
    result
}
pub(crate) fn release_asset_descriptor() -> anyhow::Result<(String, &'static str)> {
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
pub(crate) async fn download_verified_release_asset(
    client: &reqwest::Client,
    base: &str,
    binary: &str,
    asset: &str,
    extension: &str,
    destination: &Path,
    manifest: &release_verify::ReleaseManifest,
) -> anyhow::Result<()> {
    let name = format!("{binary}-{asset}{extension}");
    let expected = manifest
        .assets
        .get(&name)
        .ok_or_else(|| anyhow!("asset {name} is missing from the signed release manifest"))?;
    let bytes = client
        .get(format!("{base}/{name}"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != *expected {
        return Err(anyhow!(
            "SHA-256 mismatch for {name}: expected {expected}, got {actual}"
        ));
    }
    fs::write(destination, &bytes)
        .with_context(|| format!("write staged update {}", destination.display()))?;
    Ok(())
}
pub(crate) fn make_executable_like(path: &Path, reference: &Path) -> anyhow::Result<()> {
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
