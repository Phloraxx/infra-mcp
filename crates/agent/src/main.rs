use infra_platform::command_runner;
mod container_ops;
mod desktop_ops;
mod files;
mod git_ops;
mod identity;
mod jobs;
mod network_ops;
mod processes;
mod readiness;
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
    GatewayToAgent, LegacyDeviceHello, OperationResult, PROTOCOL_VERSION, ReconcileSnapshot,
    SignedDeviceHello,
};
use jobs::Jobs;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};
use terminal::Terminals;
use tokio::{
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
    if args.get(1).map(String::as_str) == Some("--build-info") {
        println!(
            "{}",
            serde_json::json!({"component":"infra-agent","activation_protocol":1,"version":env!("CARGO_PKG_VERSION")})
        );
        return Ok(());
    }
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

    let proof_writer = readiness::proof_writer(state_dir.clone());
    let mut delay = Duration::from_secs(1);
    loop {
        update_runtime(&state_dir, "connecting", None, None, None, None);
        match run_connection(config.clone(), context.clone(), &state_dir, &proof_writer).await {
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
    if state != "online" {
        runtime.gateway_epoch = None;
        runtime.agent_instance_id = None;
    }
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

fn mark_connection_identity(
    state_dir: &std::path::Path,
    storage: Option<&str>,
    gateway_epoch: Uuid,
    instance: Uuid,
) {
    let Ok(Some(mut runtime)) = infra_local_state::read_runtime(state_dir) else {
        return;
    };
    runtime.gateway_epoch = Some(gateway_epoch);
    runtime.agent_instance_id = Some(instance);
    if let Some(storage) = storage {
        runtime.credential_storage = Some(storage.to_string());
        runtime.credential_verified_unix_ms = Some(now_ms());
    }
    runtime.updated_unix_ms = now_ms();
    if let Err(error) = write_runtime(state_dir, &runtime) {
        warn!(%error, "failed to record credential verification state");
    }
}

async fn run_connection(
    config: Config,
    context: AgentContext,
    state_dir: &std::path::Path,
    proof_writer: &tokio::sync::watch::Sender<Option<infra_local_state::ConnectionProof>>,
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
            infra_protocol::CONNECTION_READINESS_CAPABILITY.into(),
            infra_protocol::RECONCILE_HASH_CAPABILITY.into(),
            "exec".into(),
            "files".into(),
            infra_protocol::FILE_LIST_PAGINATION_CAPABILITY.into(),
            "search".into(),
            "processes".into(),
            "jobs".into(),
            "jobs.wait_v1".into(),
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
    mark_connection_identity(
        state_dir,
        config
            .identity
            .as_ref()
            .map(|identity| identity.storage_label()),
        ack.gateway_epoch,
        agent_instance_id,
    );
    let mut readiness = readiness::Exchange::default();
    // The gateway only understands the compact frame when it acknowledged the
    // capability; an old gateway keeps receiving full snapshots.
    let compact_reconcile = ack.reconcile_hash;
    let last_reconcile_hash = Arc::new(std::sync::Mutex::new(None::<String>));

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
        .send(reconcile_message(
            &context,
            &last_reconcile_hash,
            compact_reconcile,
        ))
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
                    Ok(GatewayToAgent::ConnectionProbe(probe)) => {
                        if config.identity.is_some()
                            && let Some(reply) = readiness.reply(probe, ack.connection_generation)
                        {
                            heartbeat_tx
                                .try_send(AgentToGateway::ConnectionProbeReply(reply))
                                .map_err(|_| {
                                    anyhow!("readiness response queue full; reconnecting")
                                })?;
                            let _ = out_tx.try_send(reconcile_message(
                                &context,
                                &last_reconcile_hash,
                                compact_reconcile,
                            ));
                        }
                    }
                    Ok(GatewayToAgent::ConnectionReady(reply)) => {
                        if config.identity.is_some()
                            && let Some(probe_started_unix_ms) = readiness.confirmed(&reply)
                        {
                            let _ = proof_writer.send(Some(infra_local_state::ConnectionProof {
                                version: infra_local_state::CONNECTION_PROOF_VERSION,
                                pid: std::process::id(),
                                device_id: device_id.clone(),
                                gateway_fingerprint: infra_local_state::gateway_fingerprint(
                                    &config.gateway_url,
                                ),
                                gateway_epoch: ack.gateway_epoch,
                                agent_instance_id,
                                connection_generation: ack.connection_generation,
                                probe_started_unix_ms,
                                verified_unix_ms: now_ms(),
                            }));
                        }
                    }
                    Ok(GatewayToAgent::Request(request)) => {
                        // Never await a worker permit in the WebSocket receive loop: when
                        // all slots are busy, that would block Ping/Pong for up to a whole
                        // command timeout and cause an unnecessary device disconnect.
                        if let Ok(permit) = permits.clone().try_acquire_owned() {
                            let out = out_tx.clone();
                            let context = context.clone();
                            let last_hash = last_reconcile_hash.clone();
                            tokio::spawn(async move {
                                let response = dispatch(request, context.clone()).await;
                                drop(permit);
                                let _ = out.send(AgentToGateway::Response(response)).await;
                                // Reconcile newly finished workers after each operation.
                                let _ = out
                                    .send(reconcile_message(
                                        &context,
                                        &last_hash,
                                        compact_reconcile,
                                    ))
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
                        let _ = out_tx.try_send(reconcile_message(
                            &context,
                            &last_reconcile_hash,
                            compact_reconcile,
                        ));
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

/// Send the full snapshot when its content changed, or the compact unchanged
/// frame when the gateway acknowledged `reconcile.hash_v1` and nothing changed.
/// The hash is remembered per connection, so the first reconcile after a
/// reconnect is always a full snapshot the gateway can rebuild its view from.
fn reconcile_message(
    context: &AgentContext,
    last_hash: &std::sync::Mutex<Option<String>>,
    compact: bool,
) -> AgentToGateway {
    let snapshot = reconcile_snapshot(context);
    let hash = snapshot.content_hash();
    let mut last = last_hash.lock().expect("reconcile hash poisoned");
    if compact && last.as_deref() == Some(hash.as_str()) {
        return AgentToGateway::ReconcileUnchanged { hash };
    }
    *last = Some(hash);
    AgentToGateway::Reconcile(snapshot)
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
    let mut cmd = Command::from(command_runner::windows_shell(&command));
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("/bin/sh");
        c.args(["-lc", &command]);
        c
    };
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    match command_runner::run_async(cmd, Duration::from_millis(timeout_ms), MAX_CAPTURE_BYTES).await
    {
        Ok(output) => OperationResult::Ok {
            body: serde_json::to_value(output).expect("command output is serializable"),
        },
        Err(command_runner::CommandError::DeadlineExceeded(_)) => OperationResult::DeadlineExceeded,
        Err(command_runner::CommandError::Failed(error)) => OperationResult::Failed { error },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use infra_protocol::ExecOutput;

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

    #[cfg(windows)]
    #[tokio::test]
    async fn exec_preserves_quoted_paths_arguments_and_shell_operators() {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("quoted path with spaces.ps1");
        std::fs::write(&script, "param([string]$Value)\nWrite-Output $Value\n").unwrap();
        let command = format!(
            r#"powershell.exe -NoLogo -NoProfile -NonInteractive -File "{}" "value with spaces & literal" && echo shell-operator-ok"#,
            script.display()
        );
        // This checks quoting, not PowerShell cold-start latency on hosted CI.
        let result = exec(command, None, 30_000).await;
        let OperationResult::Ok { body } = result else {
            panic!("quoted Windows shell command failed: {result:?}");
        };
        let output: ExecOutput = serde_json::from_value(body).unwrap();
        assert_eq!(output.exit_code, Some(0), "{}", output.stderr);
        assert!(output.stdout.contains("value with spaces & literal"));
        assert!(output.stdout.contains("shell-operator-ok"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_enforces_deadline() {
        let result = exec("sleep 2".into(), None, 100).await;
        assert!(matches!(result, OperationResult::DeadlineExceeded));
    }
}
