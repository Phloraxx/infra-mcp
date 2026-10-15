use anyhow::{Context, anyhow};
use futures_util::{SinkExt, StreamExt};
use infra_platform::identity;
use infra_protocol::{
    AgentOperation, AgentRequest, AgentResponse, AgentToGateway, DeviceHello, ExecOutput,
    GatewayToAgent, OperationResult, PROTOCOL_VERSION,
};
use std::{collections::BTreeMap, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::{Semaphore, mpsc},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use uuid::Uuid;

const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
const MAX_CONCURRENT_REQUESTS: usize = 8;

#[derive(Clone)]
struct Config {
    gateway_url: String,
    token: String,
    device_id: String,
    device_name: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("infra_agent=info".parse()?),
        )
        .json()
        .init();

    let platform = identity();
    let config = Config {
        gateway_url: std::env::var("INFRA_GATEWAY_URL")
            .unwrap_or_else(|_| "ws://127.0.0.1:8788/agent".into()),
        token: std::env::var("INFRA_AGENT_TOKEN").context("INFRA_AGENT_TOKEN is required")?,
        device_id: std::env::var("INFRA_DEVICE_ID")
            .unwrap_or_else(|_| platform.hostname.to_lowercase()),
        device_name: std::env::var("INFRA_DEVICE_NAME")
            .unwrap_or_else(|_| platform.hostname.clone()),
    };

    let mut delay = Duration::from_secs(1);
    loop {
        match run_connection(config.clone()).await {
            Ok(()) => {
                warn!("gateway connection ended; reconnecting");
                delay = Duration::from_secs(1);
            }
            Err(err) => {
                warn!(%err, "gateway connection failed; reconnecting");
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        }
        tokio::time::sleep(delay).await;
    }
}

async fn run_connection(config: Config) -> anyhow::Result<()> {
    let platform = identity();
    let (socket, _) = connect_async(&config.gateway_url)
        .await
        .with_context(|| format!("connect {}", config.gateway_url))?;
    let (mut ws_tx, mut ws_rx) = socket.split();

    let hello = AgentToGateway::Hello(DeviceHello {
        protocol: PROTOCOL_VERSION,
        token: config.token.clone(),
        device_id: config.device_id.clone(),
        device_name: config.device_name.clone(),
        agent_version: env!("CARGO_PKG_VERSION").into(),
        platform: platform.platform.into(),
        arch: platform.arch.into(),
        boot_id: platform.boot_id,
        agent_instance_id: Uuid::new_v4(),
        capabilities: vec!["status".into(), "exec".into()],
        metadata: BTreeMap::from([("hostname".into(), platform.hostname)]),
    });
    ws_tx
        .send(Message::Text(serde_json::to_string(&hello)?.into()))
        .await?;

    let ack = tokio::time::timeout(Duration::from_secs(5), ws_rx.next())
        .await
        .context("hello ack timeout")?
        .ok_or_else(|| anyhow!("gateway closed before hello ack"))??;
    let ack = match ack {
        Message::Text(text) => serde_json::from_str::<GatewayToAgent>(text.as_str())?,
        _ => return Err(anyhow!("unexpected hello ack frame")),
    };
    let generation = match ack {
        GatewayToAgent::HelloAck(ack) if ack.protocol == PROTOCOL_VERSION => {
            ack.connection_generation
        }
        GatewayToAgent::HelloAck(ack) => {
            return Err(anyhow!(
                "protocol mismatch: gateway {} agent {}",
                ack.protocol,
                PROTOCOL_VERSION
            ));
        }
        _ => return Err(anyhow!("gateway did not send hello ack")),
    };
    info!(device_id=%config.device_id, generation, "connected to Infra gateway");

    let (out_tx, mut out_rx) = mpsc::channel::<AgentToGateway>(64);
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            let text = serde_json::to_string(&message)?;
            ws_tx.send(Message::Text(text.into())).await?;
        }
        Ok::<_, anyhow::Error>(())
    });

    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
    while let Some(frame) = ws_rx.next().await {
        let frame = frame?;
        match frame {
            Message::Text(text) => match serde_json::from_str::<GatewayToAgent>(text.as_str()) {
                Ok(GatewayToAgent::Request(request)) => {
                    let permit = permits.clone().acquire_owned().await?;
                    let out = out_tx.clone();
                    tokio::spawn(async move {
                        let response = dispatch(request).await;
                        drop(permit);
                        let _ = out.send(AgentToGateway::Response(response)).await;
                    });
                }
                Ok(GatewayToAgent::Ping) => {
                    out_tx.send(AgentToGateway::Pong).await?;
                }
                Ok(GatewayToAgent::HelloAck(_)) => warn!("duplicate hello ack ignored"),
                Err(err) => warn!(%err, "invalid gateway frame"),
            },
            Message::Ping(payload) => {
                // Tungstenite normally queues Pong automatically, but an explicit Pong keeps behavior obvious.
                let _ = payload;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    drop(out_tx);
    writer.abort();
    Ok(())
}

async fn dispatch(request: AgentRequest) -> AgentResponse {
    let result = match request.operation {
        AgentOperation::Status => OperationResult::Ok {
            body: serde_json::json!({
                "pid": std::process::id(),
                "protocol": PROTOCOL_VERSION,
            }),
        },
        AgentOperation::Exec {
            command,
            cwd,
            timeout_ms,
        } => {
            exec(
                command,
                cwd,
                timeout_ms.unwrap_or(30_000).clamp(100, 120_000),
            )
            .await
        }
    };
    AgentResponse {
        id: request.id,
        result,
    }
}

async fn exec(command: String, cwd: Option<String>, timeout_ms: u64) -> OperationResult {
    if command.trim().is_empty() || command.len() > 32 * 1024 {
        return OperationResult::Failed {
            error: "command must be 1..32768 bytes".into(),
        };
    }

    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd.exe");
        c.args(["/D", "/S", "/C", &command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("/bin/sh");
        c.args(["-lc", &command]);
        c
    };
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            return OperationResult::Failed {
                error: format!("spawn failed: {err}"),
            };
        }
    };
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let stdout_task = tokio::spawn(read_bounded(stdout, MAX_CAPTURE_BYTES));
    let stderr_task = tokio::spawn(read_bounded(stderr, MAX_CAPTURE_BYTES));

    let status = match tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait()).await {
        Ok(Ok(status)) => Some(status),
        Ok(Err(err)) => {
            let _ = child.kill().await;
            return OperationResult::Failed {
                error: format!("wait failed: {err}"),
            };
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return OperationResult::DeadlineExceeded;
        }
    };

    let (stdout, stdout_truncated) = stdout_task.await.unwrap_or_default();
    let (stderr, stderr_truncated) = stderr_task.await.unwrap_or_default();
    let output = ExecOutput {
        exit_code: status.and_then(|s| s.code()),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        truncated: stdout_truncated || stderr_truncated,
    };
    OperationResult::Ok {
        body: serde_json::to_value(output)
            .unwrap_or_else(|_| serde_json::json!({"error":"serialize exec output"})),
    }
}

async fn read_bounded<R>(mut reader: R, limit: usize) -> (Vec<u8>, bool)
where
    R: AsyncRead + Unpin,
{
    let mut kept = Vec::with_capacity(limit.min(64 * 1024));
    let mut buf = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        if kept.len() < limit {
            let room = limit - kept.len();
            let take = n.min(room);
            kept.extend_from_slice(&buf[..take]);
            if take < n {
                truncated = true;
            }
        } else {
            truncated = true;
        }
    }
    (kept, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn exec_captures_successful_output() {
        let result = exec("echo infra-v1-test".into(), None, 5_000).await;
        let OperationResult::Ok { body } = result else {
            panic!("expected successful exec");
        };
        let output: ExecOutput = serde_json::from_value(body).unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert!(output.stdout.contains("infra-v1-test"));
        assert!(!output.truncated);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_enforces_deadline() {
        let result = exec("sleep 2".into(), None, 100).await;
        assert!(matches!(result, OperationResult::DeadlineExceeded));
    }

    #[tokio::test]
    async fn bounded_reader_discards_after_limit_without_blocking() {
        let (mut writer, reader) = tokio::io::duplex(4096);
        let producer = tokio::spawn(async move {
            writer.write_all(&vec![b'x'; 2048]).await.unwrap();
        });
        let (bytes, truncated) = read_bounded(reader, 128).await;
        producer.await.unwrap();
        assert_eq!(bytes.len(), 128);
        assert!(truncated);
    }
}
