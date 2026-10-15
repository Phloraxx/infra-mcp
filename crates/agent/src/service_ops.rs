use crate::command_runner::{require_success, run};
use infra_protocol::{OperationResult, ServiceAction, ServiceRequest};
use serde::Serialize;
use std::{process::Command, time::Duration};

const OUTPUT_LIMIT: usize = 512 * 1024;

#[derive(Debug, Serialize)]
struct ServiceView {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    substate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
}

pub fn handle(request: ServiceRequest) -> OperationResult {
    let result = match request.action {
        ServiceAction::List { query } => list(query.as_deref()),
        ServiceAction::Status { name } => status(&name),
        ServiceAction::Start { name } => mutate("start", &name),
        ServiceAction::Stop { name } => mutate("stop", &name),
        ServiceAction::Restart { name } => mutate("restart", &name),
    };
    match result {
        Ok(body) => OperationResult::Ok { body },
        Err(error) => OperationResult::Failed { error },
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 256 || name.contains('\0') {
        Err("service name must be 1..256 characters".into())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn list(query: Option<&str>) -> Result<serde_json::Value, String> {
    let mut command = Command::new("systemctl");
    command.args([
        "--user",
        "list-units",
        "--type=service",
        "--all",
        "--no-legend",
        "--no-pager",
        "--plain",
    ]);
    let output = require_success(run(&mut command, Duration::from_secs(10), OUTPUT_LIMIT)?)?;
    let query = query.map(str::to_ascii_lowercase);
    let services = output
        .stdout
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 4 {
                return None;
            }
            let view = ServiceView {
                name: fields[0].to_string(),
                state: Some(fields[2].to_string()),
                substate: Some(fields[3].to_string()),
                description: (fields.len() > 4).then(|| fields[4..].join(" ")),
                pid: None,
            };
            if query.as_ref().is_some_and(|query| {
                !view.name.to_ascii_lowercase().contains(query)
                    && !view
                        .description
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(query)
            }) {
                None
            } else {
                Some(view)
            }
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({"scope":"user","services":services,"truncated":output.truncated}))
}

#[cfg(target_os = "linux")]
fn status(name: &str) -> Result<serde_json::Value, String> {
    validate_name(name)?;
    let mut command = Command::new("systemctl");
    command.args([
        "--user",
        "show",
        name,
        "--no-pager",
        "--property=Id,LoadState,ActiveState,SubState,Description,MainPID",
    ]);
    let output = require_success(run(&mut command, Duration::from_secs(10), OUTPUT_LIMIT)?)?;
    let mut object = serde_json::Map::new();
    for line in output.stdout.lines() {
        if let Some((key, value)) = line.split_once('=') {
            object.insert(
                key.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }
    object.insert("scope".into(), "user".into());
    Ok(object.into())
}

#[cfg(target_os = "linux")]
fn mutate(action: &str, name: &str) -> Result<serde_json::Value, String> {
    validate_name(name)?;
    let mut command = Command::new("systemctl");
    command.args(["--user", action, name]);
    let output = require_success(run(&mut command, Duration::from_secs(30), OUTPUT_LIMIT)?)?;
    Ok(
        serde_json::json!({"ok":true,"scope":"user","action":action,"service":name,"stderr":output.stderr}),
    )
}

#[cfg(target_os = "macos")]
fn list(query: Option<&str>) -> Result<serde_json::Value, String> {
    let mut command = Command::new("launchctl");
    command.arg("list");
    let output = require_success(run(&mut command, Duration::from_secs(10), OUTPUT_LIMIT)?)?;
    let query = query.map(str::to_ascii_lowercase);
    let services = output
        .stdout
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 3 {
                return None;
            }
            let view = ServiceView {
                name: fields[2].to_string(),
                state: Some(fields[1].to_string()),
                substate: None,
                description: None,
                pid: fields[0].parse().ok(),
            };
            if query
                .as_ref()
                .is_some_and(|query| !view.name.to_ascii_lowercase().contains(query))
            {
                None
            } else {
                Some(view)
            }
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({"scope":"user","services":services,"truncated":output.truncated}))
}

#[cfg(target_os = "macos")]
fn status(name: &str) -> Result<serde_json::Value, String> {
    validate_name(name)?;
    let target = format!("gui/{}/{name}", current_uid()?);
    let mut command = Command::new("launchctl");
    command.args(["print", &target]);
    let output = require_success(run(&mut command, Duration::from_secs(10), OUTPUT_LIMIT)?)?;
    let mut safe = serde_json::Map::new();
    for line in output.stdout.lines().map(str::trim) {
        for key in [
            "path",
            "state",
            "program",
            "pid",
            "runs",
            "last exit code",
            "type",
        ] {
            let prefix = format!("{key} =");
            if let Some(value) = line.strip_prefix(&prefix) {
                safe.insert(key.replace(' ', "_"), value.trim().into());
            }
        }
    }
    Ok(serde_json::json!({
        "service":name,
        "scope":"user",
        "details":safe,
        "truncated":output.truncated
    }))
}

#[cfg(target_os = "macos")]
fn mutate(action: &str, name: &str) -> Result<serde_json::Value, String> {
    validate_name(name)?;
    let target = format!("gui/{}/{name}", current_uid()?);
    let mut command = Command::new("launchctl");
    match action {
        "start" => {
            command.args(["kickstart", &target]);
        }
        "restart" => {
            command.args(["kickstart", "-k", &target]);
        }
        "stop" => {
            command.args(["kill", "TERM", &target]);
        }
        _ => return Err("unsupported launchctl action".into()),
    }
    require_success(run(&mut command, Duration::from_secs(30), OUTPUT_LIMIT)?)?;
    Ok(serde_json::json!({"ok":true,"scope":"user","action":action,"service":name}))
}

#[cfg(target_os = "macos")]
fn current_uid() -> Result<String, String> {
    let mut command = Command::new("id");
    command.arg("-u");
    let output = require_success(run(&mut command, Duration::from_secs(5), 1024)?)?;
    Ok(output.stdout.trim().to_string())
}

#[cfg(windows)]
fn list(query: Option<&str>) -> Result<serde_json::Value, String> {
    let mut command = Command::new("sc.exe");
    command.args(["query", "type=", "service", "state=", "all"]);
    let output = require_success(run(&mut command, Duration::from_secs(15), OUTPUT_LIMIT)?)?;
    let query = query.map(str::to_ascii_lowercase);
    let mut services = Vec::new();
    let mut current: Option<ServiceView> = None;
    for line in output.stdout.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix("SERVICE_NAME:") {
            if let Some(view) = current.take() {
                services.push(view);
            }
            current = Some(ServiceView {
                name: name.trim().into(),
                state: None,
                substate: None,
                description: None,
                pid: None,
            });
        } else if let Some(state) = trimmed.strip_prefix("STATE")
            && let Some((_, value)) = state.split_once(':')
            && let Some(view) = current.as_mut()
        {
            view.state = Some(value.trim().to_string());
        }
    }
    if let Some(view) = current {
        services.push(view);
    }
    if let Some(query) = query {
        services.retain(|service| service.name.to_ascii_lowercase().contains(&query));
    }
    Ok(
        serde_json::json!({"scope":"system-as-current-user","services":services,"truncated":output.truncated}),
    )
}

#[cfg(windows)]
fn status(name: &str) -> Result<serde_json::Value, String> {
    validate_name(name)?;
    let mut command = Command::new("sc.exe");
    command.args(["queryex", name]);
    let output = require_success(run(&mut command, Duration::from_secs(10), OUTPUT_LIMIT)?)?;
    Ok(serde_json::json!({"service":name,"detail":output.stdout,"truncated":output.truncated}))
}

#[cfg(windows)]
fn mutate(action: &str, name: &str) -> Result<serde_json::Value, String> {
    validate_name(name)?;
    match action {
        "start" | "stop" => {
            let mut command = Command::new("sc.exe");
            command.args([action, name]);
            require_success(run(&mut command, Duration::from_secs(30), OUTPUT_LIMIT)?)?;
        }
        "restart" => {
            let mut stop = Command::new("sc.exe");
            stop.args(["stop", name]);
            let _ = run(&mut stop, Duration::from_secs(30), OUTPUT_LIMIT);
            let mut start = Command::new("sc.exe");
            start.args(["start", name]);
            require_success(run(&mut start, Duration::from_secs(30), OUTPUT_LIMIT)?)?;
        }
        _ => return Err("unsupported service action".into()),
    }
    Ok(serde_json::json!({"ok":true,"action":action,"service":name}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_blank_service_name() {
        assert!(validate_name("   ").is_err());
    }
}
