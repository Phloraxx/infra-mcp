use super::*;

#[derive(Debug, Serialize)]
pub(crate) struct DoctorReport {
    pub(crate) healthy: bool,
    pub(crate) checks: Vec<DoctorCheck>,
}
#[derive(Debug, Serialize)]
pub(crate) struct DoctorCheck {
    pub(crate) name: &'static str,
    pub(crate) status: &'static str,
    pub(crate) detail: String,
    pub(crate) fixable: bool,
}
pub(crate) async fn doctor(state_dir: &Path) -> DoctorReport {
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
pub(crate) fn repair(state_dir: &Path) -> anyhow::Result<()> {
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
pub(crate) fn remove_native_identity_credentials(state_dir: &Path) -> anyhow::Result<()> {
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
pub(crate) fn identity_storage_check(
    state_dir: &Path,
    runtime: Option<&RuntimeStatus>,
) -> DoctorCheck {
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
pub(crate) fn check(name: &'static str, ok: bool, detail: String, fixable: bool) -> DoctorCheck {
    DoctorCheck {
        name,
        status: if ok { "ok" } else { "problem" },
        detail,
        fixable: !ok && fixable,
    }
}
pub(crate) fn cleanup_persistent_workers(state_dir: &Path) -> usize {
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

    let Ok(system) = System::new_all() else {
        eprintln!("warning: could not snapshot the process table; persistent-worker cleanup skipped");
        return 0;
    };
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
pub(crate) fn cleanup_platform_worker_registrations(state_dir: &Path) {
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
pub(crate) fn cleanup_platform_worker_registrations(_state_dir: &Path) {}
pub(crate) fn collect_worker_pids(root: &Path, meta_name: &str, key: &str, out: &mut HashSet<u32>) {
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
pub(crate) fn collect_descendants(system: &System, parent: Pid, out: &mut Vec<Pid>) {
    for (pid, process) in system.processes() {
        if process.parent() == Some(parent) {
            collect_descendants(system, *pid, out);
            out.push(*pid);
        }
    }
}
pub(crate) fn is_infra_agent_process(process: &sysinfo::Process) -> bool {
    let name = process.name().to_string_lossy().to_ascii_lowercase();
    let executable = process
        .exe()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name.contains("infra-agent") || executable.contains("infra-agent")
}
pub(crate) fn process_alive(pid: u32) -> bool {
    System::new_all()
        .map(|system| system.process(Pid::from_u32(pid)).is_some_and(is_infra_agent_process))
        .unwrap_or(false)
}
