use crate::command_runner::{require_success, run};
use infra_protocol::{ContainerAction, ContainerRequest, OperationResult};
use std::{process::Command, time::Duration};

const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;

pub fn handle(request: ContainerRequest) -> OperationResult {
    let result = match request.action {
        ContainerAction::List => list(),
        ContainerAction::Inspect { container } => inspect(&container),
        ContainerAction::Logs { container, tail } => logs(&container, tail),
        ContainerAction::Start { container } => mutate("start", &container),
        ContainerAction::Stop { container } => mutate("stop", &container),
        ContainerAction::Restart { container } => mutate("restart", &container),
    };
    match result {
        Ok(body) => OperationResult::Ok { body },
        Err(error) => OperationResult::Failed { error },
    }
}

fn validate_container(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 256 || value.contains('\0') {
        Err("container identifier must be 1..256 characters".into())
    } else {
        Ok(())
    }
}

fn docker(
    args: &[&str],
    timeout: Duration,
    limit: usize,
) -> Result<crate::command_runner::CommandOutput, String> {
    let mut command = Command::new("docker");
    command.args(args);
    require_success(run(&mut command, timeout, limit)?)
}

fn list() -> Result<serde_json::Value, String> {
    let output = docker(
        &["ps", "-a", "--no-trunc", "--format", "{{json .}}"],
        Duration::from_secs(15),
        OUTPUT_LIMIT,
    )?;
    let mut containers = Vec::new();
    for line in output.stdout.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(value) => containers.push(value),
            Err(_) => containers.push(serde_json::json!({"raw":line})),
        }
    }
    Ok(serde_json::json!({"containers":containers,"truncated":output.truncated}))
}

fn inspect(container: &str) -> Result<serde_json::Value, String> {
    validate_container(container)?;
    let output = docker(
        &["inspect", container],
        Duration::from_secs(15),
        OUTPUT_LIMIT,
    )?;
    if output.truncated {
        return Err(
            "docker inspect output exceeded the 2 MiB safety limit; use a narrower inspection path"
                .into(),
        );
    }
    let mut value: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|error| format!("docker inspect returned invalid JSON: {error}"))?;
    redact_json(&mut value, None);
    Ok(value)
}

fn redact_json(value: &mut serde_json::Value, key: Option<&str>) {
    if key.is_some_and(is_sensitive_name) {
        *value = serde_json::Value::String("<redacted>".into());
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            for (child_key, child) in map.iter_mut() {
                if child_key.eq_ignore_ascii_case("Env") {
                    redact_env_array(child);
                } else {
                    redact_json(child, Some(child_key));
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json(item, key);
            }
        }
        _ => {}
    }
}

fn redact_env_array(value: &mut serde_json::Value) {
    let serde_json::Value::Array(items) = value else {
        return;
    };
    for item in items {
        let serde_json::Value::String(entry) = item else {
            continue;
        };
        if let Some((name, _)) = entry.split_once('=')
            && is_sensitive_name(name)
        {
            *entry = format!("{name}=<redacted>");
        }
    }
}

fn is_sensitive_name(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase().replace('-', "_");
    [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "private_key",
        "access_key",
        "credential",
        "client_secret",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn logs(container: &str, tail: Option<usize>) -> Result<serde_json::Value, String> {
    validate_container(container)?;
    let tail = tail.unwrap_or(200).clamp(1, 1000).to_string();
    let output = docker(
        &["logs", "--timestamps", "--tail", &tail, container],
        Duration::from_secs(20),
        OUTPUT_LIMIT,
    )?;
    Ok(serde_json::json!({
        "container":container,
        "stdout":output.stdout,
        "stderr":output.stderr,
        "truncated":output.truncated
    }))
}

fn mutate(action: &str, container: &str) -> Result<serde_json::Value, String> {
    validate_container(container)?;
    let output = docker(&[action, container], Duration::from_secs(60), OUTPUT_LIMIT)?;
    Ok(serde_json::json!({
        "ok":true,
        "action":action,
        "container":container,
        "result":output.stdout.trim(),
        "stderr":output.stderr,
        "truncated":output.truncated
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_container_identifier() {
        assert!(validate_container("web-1").is_ok());
        assert!(validate_container("").is_err());
    }

    #[test]
    fn inspect_redaction_removes_sensitive_environment_values() {
        let mut value = serde_json::json!({
            "Config": {
                "Env": ["NORMAL=value", "API_TOKEN=supersecret"],
                "Password": "also-secret"
            }
        });
        redact_json(&mut value, None);
        assert_eq!(value["Config"]["Env"][0], "NORMAL=value");
        assert_eq!(value["Config"]["Env"][1], "API_TOKEN=<redacted>");
        assert_eq!(value["Config"]["Password"], "<redacted>");
    }
}
