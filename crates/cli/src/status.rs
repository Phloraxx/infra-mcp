use super::*;

#[derive(Debug, Serialize)]
pub(crate) struct LocalStatus {
    pub(crate) state_dir: String,
    pub(crate) configured: bool,
    pub(crate) enrolled: bool,
    pub(crate) process_alive: bool,
    pub(crate) connection: readiness::ConnectionStatus,
    pub(crate) startup: Option<startup::StartupStatus>,
    pub(crate) config: Option<LocalConfig>,
    pub(crate) runtime: Option<RuntimeStatus>,
    pub(crate) update_status: Option<update_handoff::UpdateStatus>,
}
pub(crate) fn collect_status(state_dir: &Path, check_startup: bool) -> anyhow::Result<LocalStatus> {
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
pub(crate) fn gateway_health_url(gateway_ws_url: &str) -> anyhow::Result<Url> {
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
pub(crate) fn classify_network_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return "gateway health check timed out".into();
    }
    if error.is_connect() {
        return format!("cannot connect to gateway: {error}");
    }
    format!("gateway health check failed: {error}")
}
pub(crate) fn print_status(status: &LocalStatus) {
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
pub(crate) fn print_doctor(report: &DoctorReport) {
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
pub(crate) fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
