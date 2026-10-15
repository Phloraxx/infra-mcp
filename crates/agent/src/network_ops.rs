use crate::command_runner::{require_success, run};
use infra_protocol::{NetworkAction, NetworkRequest, OperationResult};
use serde::Serialize;
use std::{
    net::SocketAddr,
    process::Command,
    time::{Duration, Instant},
};
use tokio::net::TcpStream;

const MAX_LISTENER_OUTPUT: usize = 1024 * 1024;

#[derive(Debug, Serialize)]
struct ResolveOutput {
    host: String,
    addresses: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TcpCheckOutput {
    host: String,
    port: u16,
    reachable: bool,
    elapsed_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    peer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ListenerOutput {
    source: &'static str,
    entries: Vec<ListenerEntry>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct ListenerEntry {
    protocol: String,
    local: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    process: Option<String>,
    raw: String,
}

pub async fn handle(request: NetworkRequest) -> OperationResult {
    let result = match request.action {
        NetworkAction::Resolve { host } => resolve(host).await.and_then(json),
        NetworkAction::TcpCheck {
            host,
            port,
            timeout_ms,
        } => tcp_check(host, port, timeout_ms).await.and_then(json),
        NetworkAction::Listeners => tokio::task::spawn_blocking(listeners)
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result)
            .and_then(json),
    };
    match result {
        Ok(body) => OperationResult::Ok { body },
        Err(error) => OperationResult::Failed { error },
    }
}

fn json(value: impl Serialize) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

async fn resolve(host: String) -> Result<ResolveOutput, String> {
    validate_host(&host)?;
    let mut addresses = tokio::net::lookup_host((host.as_str(), 0))
        .await
        .map_err(|error| format!("resolve failed: {error}"))?
        .map(|address| address.ip().to_string())
        .collect::<Vec<_>>();
    addresses.sort();
    addresses.dedup();
    Ok(ResolveOutput { host, addresses })
}

async fn tcp_check(
    host: String,
    port: u16,
    timeout_ms: Option<u64>,
) -> Result<TcpCheckOutput, String> {
    validate_host(&host)?;
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(3_000).clamp(100, 30_000));
    let start = Instant::now();
    let addresses = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|error| format!("resolve failed: {error}"))?
        .collect::<Vec<SocketAddr>>();
    if addresses.is_empty() {
        return Err("hostname resolved to no addresses".into());
    }
    let mut last_error = None;
    for address in addresses {
        match tokio::time::timeout(timeout, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => {
                return Ok(TcpCheckOutput {
                    host,
                    port,
                    reachable: true,
                    elapsed_ms: start.elapsed().as_millis(),
                    peer: stream.peer_addr().ok().map(|address| address.to_string()),
                    error: None,
                });
            }
            Ok(Err(error)) => last_error = Some(error.to_string()),
            Err(_) => {
                last_error = Some(format!(
                    "connection timed out after {}ms",
                    timeout.as_millis()
                ))
            }
        }
    }
    Ok(TcpCheckOutput {
        host,
        port,
        reachable: false,
        elapsed_ms: start.elapsed().as_millis(),
        peer: None,
        error: last_error,
    })
}

fn validate_host(host: &str) -> Result<(), String> {
    if host.trim().is_empty() || host.len() > 253 || host.chars().any(char::is_whitespace) {
        Err("host must be a non-empty hostname/IP without whitespace".into())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn listeners() -> Result<ListenerOutput, String> {
    let mut command = Command::new("ss");
    command.args(["-H", "-lntup"]);
    let output = require_success(run(
        &mut command,
        Duration::from_secs(10),
        MAX_LISTENER_OUTPUT,
    )?)?;
    Ok(ListenerOutput {
        source: "ss",
        entries: parse_linux_ss(&output.stdout),
        truncated: output.truncated,
    })
}

#[cfg(target_os = "macos")]
fn listeners() -> Result<ListenerOutput, String> {
    let mut command = Command::new("lsof");
    command.args(["-nP", "-iTCP", "-sTCP:LISTEN"]);
    let output = require_success(run(
        &mut command,
        Duration::from_secs(10),
        MAX_LISTENER_OUTPUT,
    )?)?;
    Ok(ListenerOutput {
        source: "lsof",
        entries: parse_macos_lsof(&output.stdout),
        truncated: output.truncated,
    })
}

#[cfg(windows)]
fn listeners() -> Result<ListenerOutput, String> {
    let mut command = Command::new("netstat.exe");
    command.args(["-ano", "-p", "tcp"]);
    let output = require_success(run(
        &mut command,
        Duration::from_secs(10),
        MAX_LISTENER_OUTPUT,
    )?)?;
    Ok(ListenerOutput {
        source: "netstat",
        entries: parse_windows_netstat(&output.stdout),
        truncated: output.truncated,
    })
}

#[cfg(target_os = "linux")]
fn parse_linux_ss(text: &str) -> Vec<ListenerEntry> {
    text.lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 5 {
                return None;
            }
            let process_blob = fields
                .get(6..)
                .map(|parts| parts.join(" "))
                .unwrap_or_default();
            Some(ListenerEntry {
                protocol: fields[0].to_string(),
                local: fields[4].to_string(),
                pid: extract_pid(&process_blob),
                process: extract_quoted_process(&process_blob),
                raw: line.to_string(),
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn extract_pid(value: &str) -> Option<u32> {
    let index = value.find("pid=")? + 4;
    value[index..]
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
fn extract_quoted_process(value: &str) -> Option<String> {
    let start = value.find("((\"")? + 3;
    let end = value[start..].find('"')? + start;
    Some(value[start..end].to_string())
}

#[cfg(target_os = "macos")]
fn parse_macos_lsof(text: &str) -> Vec<ListenerEntry> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 9 {
                return None;
            }
            Some(ListenerEntry {
                protocol: "tcp".into(),
                local: fields[8].trim_end_matches(" (LISTEN)").to_string(),
                pid: fields.get(1).and_then(|value| value.parse().ok()),
                process: fields.first().map(|value| (*value).to_string()),
                raw: line.to_string(),
            })
        })
        .collect()
}

#[cfg(windows)]
fn parse_windows_netstat(text: &str) -> Vec<ListenerEntry> {
    text.lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 5
                || !fields[0].eq_ignore_ascii_case("TCP")
                || fields[3] != "LISTENING"
            {
                return None;
            }
            Some(ListenerEntry {
                protocol: "tcp".into(),
                local: fields[1].to_string(),
                pid: fields[4].parse().ok(),
                process: None,
                raw: line.trim().to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn localhost_resolves() {
        let result = resolve("localhost".into()).await.unwrap();
        assert!(!result.addresses.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_ss_listener() {
        let parsed = parse_linux_ss(
            "tcp LISTEN 0 4096 127.0.0.1:8080 0.0.0.0:* users:((\"server\",pid=42,fd=3))",
        );
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].local, "127.0.0.1:8080");
        assert_eq!(parsed[0].pid, Some(42));
        assert_eq!(parsed[0].process.as_deref(), Some("server"));
    }
}
