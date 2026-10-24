mod command_runner;
mod container_ops;
mod desktop_ops;
mod files;
mod git_ops;
mod identity;
mod jobs;
mod network_ops;
mod processes;
mod search;
mod service_ops;
mod system_ops;
mod terminal;
mod transfer;
mod worker_launch;

use anyhow::{Context, anyhow};
use futures_util::{SinkExt, StreamExt};
use identity::DeviceIdentity;
use infra_local_state::{RuntimeStatus, now_ms, read_config, write_runtime};
use infra_platform::identity as platform_identity;
use infra_protocol::{
    AgentOperation, AgentRequest, AgentResponse, AgentToGateway, DeviceAuthStart, DeviceHello,
    ExecOutput, GatewayToAgent, LegacyDeviceHello, OperationResult, PROTOCOL_VERSION,
    ReconcileSnapshot, SignedDeviceHello,
};
use jobs::Jobs;
use std::{collections::BTreeMap, path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use terminal::Terminals;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::{Semaphore, mpsc},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use transfer::Transfers;
use uuid::Uuid;

const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
const MAX_CONCURRENT_REQUESTS: usize = 8;
const GATEWAY_IDLE_TIMEOUT: Duration = Duration::from_secs(40);

#[derive(Clone)]
struct Config {
    gateway_url: String,
    identity: Option<DeviceIdentity>,
    legacy_token: Option<String>,
    legacy_device_id: String,
    device_name: String,
}

#[derive(Clone)]
struct AgentContext {
    jobs: Jobs,
    terminals: Terminals,
    transfers: Transfers,
}

#[derive(Debug)]
struct StartupArgs {
    state_dir: PathBuf,
    enroll_only: bool,
    pair_only: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("--job-worker") {
        let dir = args
            .get(2)
            .ok_or_else(|| anyhow!("--job-worker requires a directory"))?;
        return jobs::run_worker(PathBuf::from(dir).as_path());
    }
    if args.get(1).map(String::as_str) == Some("--pty-worker") {
        let dir = args
            .get(2)
            .ok_or_else(|| anyhow!("--pty-worker requires a directory"))?;
        return terminal::run_worker(PathBuf::from(dir).as_path());
    }
    let startup = parse_startup_args(&args[1..])?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("infra_agent=info".parse()?),
        )
        .json()
        .init();

    let platform = platform_identity();
    let state_dir = startup.state_dir;
    identity::ensure_state_dir(&state_dir)?;
    let disk_config = read_config(&state_dir)?;
    let gateway_url = std::env::var("INFRA_GATEWAY_URL")
        .ok()
        .or_else(|| {
            disk_config
                .as_ref()
                .map(|config| config.gateway_url.clone())
        })
        .unwrap_or_else(|| "ws://127.0.0.1:8788/agent".into());
    let requested_name = std::env::var("INFRA_DEVICE_NAME")
        .ok()
        .or_else(|| {
            disk_config
                .as_ref()
                .and_then(|config| config.device_name.clone())
        })
        .unwrap_or_else(|| platform.hostname.clone());
    update_runtime(&state_dir, "starting", None, None, None, None);

    if startup.pair_only {
        identity::pair_with_owner(
            &state_dir,
            &gateway_url,
            &platform.hostname,
            platform.platform,
            platform.arch,
            env!("CARGO_PKG_VERSION"),
        )
        .await?;
        return Ok(());
    }

    let enrollment_code = std::env::var("INFRA_ENROLLMENT_CODE").ok();
    let device_identity = match identity::load_or_enroll(
        &state_dir,
        &gateway_url,
        enrollment_code.as_deref(),
        &requested_name,
        platform.platform,
        platform.arch,
        env!("CARGO_PKG_VERSION"),
    )
    .await
    {
        Ok(identity) => identity,
        Err(error) => {
            update_runtime(
                &state_dir,
                "enrollment_failed",
                Some(error.to_string()),
                None,
                None,
                None,
            );
            return Err(error);
        }
    };

    if startup.enroll_only {
        let enrolled = device_identity
            .as_ref()
            .ok_or_else(|| anyhow!("device is not enrolled; provide INFRA_ENROLLMENT_CODE"))?;
        update_runtime(
            &state_dir,
            "enrolled",
            None,
            Some(enrolled.device_id.clone()),
            Some(enrolled.device_name.clone()),
            None,
        );
        return Ok(());
    }

    let legacy_token = std::env::var("INFRA_AGENT_TOKEN").ok();
    if device_identity.is_none() && legacy_token.is_none() {
        update_runtime(&state_dir, "needs_enrollment", None, None, None, None);
        anyhow::bail!("device is not enrolled; set INFRA_ENROLLMENT_CODE for first run");
    }

    let config = Config {
        gateway_url,
        identity: device_identity,
        legacy_token,
        legacy_device_id: std::env::var("INFRA_DEVICE_ID")
            .unwrap_or_else(|_| platform.hostname.to_lowercase()),
        device_name: requested_name,
    };
    let context = AgentContext {
        jobs: Jobs::new(&state_dir, platform.boot_id.clone())?,
        terminals: Terminals::new(&state_dir, platform.boot_id.clone())?,
        transfers: Transfers::new(&state_dir, platform.boot_id.clone())?,
    };

    let mut delay = Duration::from_secs(1);
    loop {
        update_runtime(&state_dir, "connecting", None, None, None, None);
        match run_connection(config.clone(), context.clone(), &state_dir).await {
            Ok(()) => {
                warn!("gateway connection ended; reconnecting");
                update_runtime(&state_dir, "reconnecting", None, None, None, None);
                delay = Duration::from_secs(1);
            }
            Err(err) => {
                warn!(%err, "gateway connection failed; reconnecting");
                update_runtime(
                    &state_dir,
                    "reconnecting",
                    Some(err.to_string()),
                    None,
                    None,
                    None,
                );
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        }
        tokio::time::sleep(delay).await;
    }
}

fn parse_startup_args(args: &[String]) -> anyhow::Result<StartupArgs> {
    let mut state_dir = identity::default_state_dir();
    let mut enroll_only = false;
    let mut pair_only = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--state-dir" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| anyhow!("--state-dir requires a path"))?;
                state_dir = PathBuf::from(value);
            }
            "--enroll-only" => enroll_only = true,
            "--pair-only" => pair_only = true,
            other => return Err(anyhow!("unknown agent argument {other:?}")),
        }
        index += 1;
    }
    Ok(StartupArgs {
        state_dir,
        enroll_only,
        pair_only,
    })
}

fn update_runtime(
    state_dir: &std::path::Path,
    state: &str,
    last_error: Option<String>,
    device_id: Option<String>,
    device_name: Option<String>,
    generation: Option<u64>,
) {
    let mut runtime = infra_local_state::read_runtime(state_dir)
        .ok()
        .flatten()
        .unwrap_or_else(|| RuntimeStatus::new(state));
    runtime.pid = std::process::id();
    runtime.state = state.into();
    runtime.updated_unix_ms = now_ms();
    if let Some(error) = last_error {
        runtime.last_error = Some(error);
    } else if matches!(state, "online" | "connecting" | "enrolled") {
        runtime.last_error = None;
    }
    if device_id.is_some() {
        runtime.device_id = device_id;
    }
    if device_name.is_some() {
        runtime.device_name = device_name;
    }
    if generation.is_some() {
        runtime.connection_generation = generation;
    }
    if state == "online" {
        runtime.last_connected_unix_ms = Some(now_ms());
    }
    if let Err(error) = write_runtime(state_dir, &runtime) {
        warn!(%error, "failed to write runtime status");
    }
}

fn mark_credential_verified(state_dir: &std::path::Path, storage: &str) {
    let Ok(Some(mut runtime)) = infra_local_state::read_runtime(state_dir) else {
        return;
    };
    runtime.credential_storage = Some(storage.to_string());
    runtime.credential_verified_unix_ms = Some(now_ms());
    runtime.updated_unix_ms = now_ms();
    if let Err(error) = write_runtime(state_dir, &runtime) {
        warn!(%error, "failed to record credential verification state");
    }
}

async fn run_connection(
    config: Config,
    context: AgentContext,
    state_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let platform = platform_identity();
    let agent_instance_id = Uuid::new_v4();
    let (socket, _) = connect_async(&config.gateway_url)
        .await
        .with_context(|| format!("connect {}", config.gateway_url))?;
    let (mut ws_tx, mut ws_rx) = socket.split();

    let (device_id, device_name) = config
        .identity
        .as_ref()
        .map(|identity| (identity.device_id.clone(), identity.device_name.clone()))
        .unwrap_or_else(|| (config.legacy_device_id.clone(), config.device_name.clone()));
    let capabilities = {
        let capabilities = vec![
            "status".into(),
            "exec".into(),
            "files".into(),
            "search".into(),
            "processes".into(),
            "jobs".into(),
            "terminal".into(),
            "transfer".into(),
            "system".into(),
            "network".into(),
            "services".into(),
            "containers".into(),
            "git".into(),
        ];
        #[cfg(any(target_os = "macos", windows))]
        {
            let mut capabilities = capabilities;
            capabilities.push("desktop".into());
            capabilities
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            capabilities
        }
    };
    let hello = DeviceHello {
        protocol: PROTOCOL_VERSION,
        device_id: device_id.clone(),
        device_name,
        agent_version: env!("CARGO_PKG_VERSION").into(),
        platform: platform.platform.into(),
        arch: platform.arch.into(),
        boot_id: platform.boot_id.clone(),
        agent_instance_id,
        capabilities,
        metadata: BTreeMap::from([("hostname".into(), platform.hostname)]),
    };

    if let Some(identity) = &config.identity {
        send_agent(
            &mut ws_tx,
            &AgentToGateway::AuthStart(DeviceAuthStart {
                protocol: PROTOCOL_VERSION,
                device_id: identity.device_id.clone(),
                agent_instance_id,
            }),
        )
        .await?;
        let challenge = match next_gateway_message(&mut ws_rx, Duration::from_secs(5)).await? {
            GatewayToAgent::AuthChallenge(challenge) => challenge,
            other => return Err(anyhow!("expected auth challenge, got {other:?}")),
        };
        let signature = identity.sign_hello(&challenge.nonce, challenge.gateway_epoch, &hello);
        send_agent(
            &mut ws_tx,
            &AgentToGateway::SignedHello(SignedDeviceHello {
                hello: hello.clone(),
                signature,
            }),
        )
        .await?;
    } else {
        let token = config
            .legacy_token
            .clone()
            .ok_or_else(|| anyhow!("legacy token missing"))?;
        send_agent(
            &mut ws_tx,
            &AgentToGateway::LegacyHello(LegacyDeviceHello {
                hello: hello.clone(),
                token,
            }),
        )
        .await?;
    }

    let ack = match next_gateway_message(&mut ws_rx, Duration::from_secs(5)).await? {
        GatewayToAgent::HelloAck(ack) if ack.protocol == PROTOCOL_VERSION => ack,
        GatewayToAgent::HelloAck(ack) => {
            return Err(anyhow!(
                "protocol mismatch: gateway {} agent {}",
                ack.protocol,
                PROTOCOL_VERSION
            ));
        }
        other => return Err(anyhow!("gateway did not send hello ack: {other:?}")),
    };
    info!(device_id=%device_id, generation=ack.connection_generation, "connected to Infra gateway");
    update_runtime(
        state_dir,
        "online",
        None,
        Some(device_id.clone()),
        Some(hello.device_name.clone()),
        Some(ack.connection_generation),
    );
    if let Some(identity) = &config.identity {
        mark_credential_verified(state_dir, identity.storage_label());
    }

    let (out_tx, mut out_rx) = mpsc::channel::<AgentToGateway>(64);
    let (heartbeat_tx, mut heartbeat_rx) = mpsc::channel::<AgentToGateway>(8);
    let writer = tokio::spawn(async move {
        loop {
            // Liveness must not queue behind large result/reconciliation bursts.
            let message = tokio::select! {
                biased;
                Some(message) = heartbeat_rx.recv() => message,
                Some(message) = out_rx.recv() => message,
                else => break,
            };
            let text = serde_json::to_string(&message)?;
            ws_tx.send(Message::Text(text.into())).await?;
        }
        Ok::<_, anyhow::Error>(())
    });

    out_tx
        .send(AgentToGateway::Reconcile(reconcile_snapshot(&context)))
        .await?;

    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
    loop {
        let frame = tokio::time::timeout(GATEWAY_IDLE_TIMEOUT, ws_rx.next())
            .await
            .context("gateway idle timeout; reconnecting stale connection")?;
        let Some(frame) = frame else {
            break;
        };
        let frame = frame?;
        match frame {
            Message::Text(text) => {
                match serde_json::from_str::<GatewayToAgent>(text.as_str()) {
                    Ok(GatewayToAgent::Request(request)) => {
                        // Never await a worker permit in the WebSocket receive loop: when
                        // all slots are busy, that would block Ping/Pong for up to a whole
                        // command timeout and cause an unnecessary device disconnect.
                        if let Ok(permit) = permits.clone().try_acquire_owned() {
                            let out = out_tx.clone();
                            let context = context.clone();
                            tokio::spawn(async move {
                                let response = dispatch(request, context.clone()).await;
                                drop(permit);
                                let _ = out.send(AgentToGateway::Response(response)).await;
                                // Reconcile newly finished workers after each operation.
                                let _ = out
                                    .send(AgentToGateway::Reconcile(reconcile_snapshot(&context)))
                                    .await;
                            });
                        } else {
                            out_tx.try_send(AgentToGateway::Response(AgentResponse {
                                id: request.id,
                                result: OperationResult::Failed {
                                    error: "agent_busy: concurrent request limit reached; retry later".into(),
                                },
                            })).map_err(|_| anyhow::anyhow!("agent response channel full; reconnecting"))?;
                        }
                    }
                    Ok(GatewayToAgent::Ping) => {
                        heartbeat_tx.try_send(AgentToGateway::Pong).map_err(|_| {
                            anyhow::anyhow!("heartbeat response queue full; reconnecting")
                        })?;
                        // Reconcile is opportunistic: no work snapshot is worth losing liveness.
                        let _ = out_tx
                            .try_send(AgentToGateway::Reconcile(reconcile_snapshot(&context)));
                    }
                    Ok(GatewayToAgent::HelloAck(_)) | Ok(GatewayToAgent::AuthChallenge(_)) => {
                        warn!("unexpected handshake frame ignored")
                    }
                    Err(err) => warn!(%err, "invalid gateway frame"),
                }
            }
            Message::Ping(_) => {}
            Message::Close(_) => break,
            _ => {}
        }
    }
    drop(out_tx);
    writer.abort();
    Ok(())
}

fn reconcile_snapshot(context: &AgentContext) -> ReconcileSnapshot {
    let mut items = context.jobs.reconcile_items();
    items.extend(context.terminals.reconcile_items());
    items.extend(context.transfers.reconcile_items());
    ReconcileSnapshot { items }
}

async fn send_agent<S>(sink: &mut S, message: &AgentToGateway) -> anyhow::Result<()>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    sink.send(Message::Text(serde_json::to_string(message)?.into()))
        .await
        .map_err(anyhow::Error::new)
}

async fn next_gateway_message<S>(
    stream: &mut S,
    timeout: Duration,
) -> anyhow::Result<GatewayToAgent>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let frame = tokio::time::timeout(timeout, stream.next())
        .await
        .context("gateway handshake timeout")?
        .ok_or_else(|| anyhow!("gateway closed during handshake"))??;
    match frame {
        Message::Text(text) => Ok(serde_json::from_str(text.as_str())?),
        _ => Err(anyhow!("unexpected handshake frame")),
    }
}

async fn dispatch(request: AgentRequest, context: AgentContext) -> AgentResponse {
    let result = match request.operation {
        AgentOperation::Status => OperationResult::Ok {
            body: serde_json::json!({"pid": std::process::id(), "protocol": PROTOCOL_VERSION}),
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
        AgentOperation::Files(request) => {
            tokio::task::spawn_blocking(move || files::handle(request))
                .await
                .unwrap_or_else(|e| OperationResult::Failed {
                    error: e.to_string(),
                })
        }
        AgentOperation::Search(request) => {
            tokio::task::spawn_blocking(move || search::handle(request))
                .await
                .unwrap_or_else(|e| OperationResult::Failed {
                    error: e.to_string(),
                })
        }
        AgentOperation::Processes(request) => {
            tokio::task::spawn_blocking(move || processes::handle(request))
                .await
                .unwrap_or_else(|e| OperationResult::Failed {
                    error: e.to_string(),
                })
        }
        AgentOperation::Jobs(request) => {
            let jobs = context.jobs.clone();
            tokio::task::spawn_blocking(move || jobs.handle(request))
                .await
                .unwrap_or_else(|e| OperationResult::Failed {
                    error: e.to_string(),
                })
        }
        AgentOperation::Terminal(request) => {
            let terminals = context.terminals.clone();
            tokio::task::spawn_blocking(move || terminals.handle(request))
                .await
                .unwrap_or_else(|e| OperationResult::Failed {
                    error: e.to_string(),
                })
        }
        AgentOperation::Transfer(request) => {
            let transfers = context.transfers.clone();
            tokio::task::spawn_blocking(move || transfers.handle(request))
                .await
                .unwrap_or_else(|e| OperationResult::Failed {
                    error: e.to_string(),
                })
        }
        AgentOperation::System(request) => {
            tokio::task::spawn_blocking(move || system_ops::handle(request))
                .await
                .unwrap_or_else(|e| OperationResult::Failed {
                    error: e.to_string(),
                })
        }
        AgentOperation::Network(request) => network_ops::handle(request).await,
        AgentOperation::Services(request) => {
            tokio::task::spawn_blocking(move || service_ops::handle(request))
                .await
                .unwrap_or_else(|e| OperationResult::Failed {
                    error: e.to_string(),
                })
        }
        AgentOperation::Containers(request) => {
            tokio::task::spawn_blocking(move || container_ops::handle(request))
                .await
                .unwrap_or_else(|e| OperationResult::Failed {
                    error: e.to_string(),
                })
        }
        AgentOperation::Git(request) => {
            tokio::task::spawn_blocking(move || git_ops::handle(request))
                .await
                .unwrap_or_else(|e| OperationResult::Failed {
                    error: e.to_string(),
                })
        }
        AgentOperation::Desktop(request) => {
            tokio::task::spawn_blocking(move || desktop_ops::handle(request))
                .await
                .unwrap_or_else(|e| OperationResult::Failed {
                    error: e.to_string(),
                })
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
