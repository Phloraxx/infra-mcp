use anyhow::Context;
use axum::{
    Json, Router,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use infra_protocol::{
    AgentOperation, AgentRequest, AgentResponse, AgentToGateway, DEFAULT_CONTROL_FRAME_LIMIT,
    DeviceHello, ExecOutput, GatewayToAgent, HelloAck, OperationResult, PROTOCOL_VERSION,
};
use rmcp::{
    Json as McpJson,
    handler::server::wrapper::Parameters,
    tool, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    agent_token: Arc<str>,
    gateway_epoch: Uuid,
    next_generation: Arc<AtomicU64>,
    devices: Arc<RwLock<HashMap<String, DeviceConnection>>>,
    pending: Arc<Mutex<HashMap<Uuid, oneshot::Sender<AgentResponse>>>>,
}

impl AppState {
    fn new(agent_token: String) -> Self {
        Self {
            agent_token: Arc::from(agent_token),
            gateway_epoch: Uuid::new_v4(),
            next_generation: Arc::new(AtomicU64::new(1)),
            devices: Arc::new(RwLock::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Clone)]
struct DeviceConnection {
    hello: DeviceHello,
    generation: u64,
    connected_at: SystemTime,
    tx: mpsc::Sender<GatewayToAgent>,
}

#[derive(Debug, Serialize)]
struct Health<'a> {
    status: &'a str,
    service: &'a str,
    protocol: u16,
    gateway_epoch: Uuid,
}

#[derive(Debug, Serialize)]
struct DeviceView {
    device_id: String,
    device_name: String,
    agent_version: String,
    platform: String,
    arch: String,
    boot_id: String,
    agent_instance_id: Uuid,
    capabilities: Vec<String>,
    connection_generation: u64,
    connected_seconds: u64,
    online: bool,
}

#[derive(Debug, Deserialize)]
struct ExecRequestBody {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug)]
enum ExecDispatchError {
    BadRequest(&'static str),
    Offline,
    ConnectionClosed,
    ResponseClosed,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
struct McpDeviceView {
    device_id: String,
    device_name: String,
    agent_version: String,
    platform: String,
    arch: String,
    capabilities: Vec<String>,
    connection_generation: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpExecInput {
    device: String,
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct McpExecOutput {
    request_id: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Clone)]
struct InfraMcp {
    state: AppState,
}

impl InfraMcp {
    fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[tool_router(server_handler)]
impl InfraMcp {
    #[tool(
        name = "devices",
        description = "List currently connected Infra devices and their capabilities."
    )]
    async fn devices(&self) -> McpJson<Vec<McpDeviceView>> {
        let devices = self.state.devices.read().await;
        McpJson(
            devices
                .values()
                .map(|d| McpDeviceView {
                    device_id: d.hello.device_id.clone(),
                    device_name: d.hello.device_name.clone(),
                    agent_version: d.hello.agent_version.clone(),
                    platform: d.hello.platform.clone(),
                    arch: d.hello.arch.clone(),
                    capabilities: d.hello.capabilities.clone(),
                    connection_generation: d.generation,
                })
                .collect(),
        )
    }

    #[tool(
        name = "exec",
        description = "Run a bounded one-shot command on an online Infra device as the agent user."
    )]
    async fn exec(
        &self,
        Parameters(input): Parameters<McpExecInput>,
    ) -> Result<McpJson<McpExecOutput>, String> {
        let response = execute_on_device(
            &self.state,
            input.device,
            input.command,
            input.cwd,
            input.timeout_ms,
        )
        .await
        .map_err(|e| format!("{e:?}"))?;

        let mut out = McpExecOutput {
            request_id: response.id.to_string(),
            status: "unknown".into(),
            exit_code: None,
            stdout: None,
            stderr: None,
            truncated: None,
            error: None,
        };
        match response.result {
            OperationResult::Ok { body } => {
                out.status = "ok".into();
                match serde_json::from_value::<ExecOutput>(body) {
                    Ok(exec) => {
                        out.exit_code = exec.exit_code;
                        out.stdout = Some(exec.stdout);
                        out.stderr = Some(exec.stderr);
                        out.truncated = Some(exec.truncated);
                    }
                    Err(err) => out.error = Some(format!("invalid exec response: {err}")),
                }
            }
            OperationResult::Failed { error } => {
                out.status = "failed".into();
                out.error = Some(error);
            }
            OperationResult::Canceled { error } => {
                out.status = "canceled".into();
                out.error = error;
            }
            OperationResult::DeadlineExceeded => out.status = "deadline_exceeded".into(),
            OperationResult::OutcomeUnknown { error } => {
                out.status = "outcome_unknown".into();
                out.error = Some(error);
            }
            OperationResult::Unsupported { operation } => {
                out.status = "unsupported".into();
                out.error = Some(operation);
            }
            OperationResult::Unauthorized => out.status = "unauthorized".into(),
        }
        Ok(McpJson(out))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("infra_gateway=info".parse()?),
        )
        .json()
        .init();

    let agent_token =
        std::env::var("INFRA_AGENT_TOKEN").context("INFRA_AGENT_TOKEN is required")?;
    if agent_token.len() < 16 {
        anyhow::bail!("INFRA_AGENT_TOKEN must be at least 16 characters for development");
    }

    let bind: SocketAddr = std::env::var("INFRA_GATEWAY_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8788".into())
        .parse()
        .context("invalid INFRA_GATEWAY_BIND")?;

    let state = AppState::new(agent_token);
    let app = build_app(state.clone());

    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!(%bind, gateway_epoch=%state.gateway_epoch, "Infra v1 gateway listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_app(state: AppState) -> Router {
    let mcp_state = state.clone();
    let mcp_service: StreamableHttpService<InfraMcp, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(InfraMcp::new(mcp_state.clone())),
            Default::default(),
            StreamableHttpServerConfig::default()
                .with_legacy_session_mode(false)
                .with_json_response(true),
        );

    Router::new()
        .nest_service("/mcp", mcp_service)
        .route("/healthz", get(health))
        .route("/api/devices", get(list_devices))
        .route("/api/devices/{device_id}/exec", post(exec_device))
        .route("/agent", get(agent_upgrade))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<Health<'static>> {
    Json(Health {
        status: "ok",
        service: "infra-gateway",
        protocol: PROTOCOL_VERSION,
        gateway_epoch: state.gateway_epoch,
    })
}

async fn list_devices(State(state): State<AppState>) -> Json<Vec<DeviceView>> {
    let now = SystemTime::now();
    let devices = state.devices.read().await;
    Json(
        devices
            .values()
            .map(|d| DeviceView {
                device_id: d.hello.device_id.clone(),
                device_name: d.hello.device_name.clone(),
                agent_version: d.hello.agent_version.clone(),
                platform: d.hello.platform.clone(),
                arch: d.hello.arch.clone(),
                boot_id: d.hello.boot_id.clone(),
                agent_instance_id: d.hello.agent_instance_id,
                capabilities: d.hello.capabilities.clone(),
                connection_generation: d.generation,
                connected_seconds: now
                    .duration_since(d.connected_at)
                    .unwrap_or_default()
                    .as_secs(),
                online: true,
            })
            .collect(),
    )
}

async fn exec_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(body): Json<ExecRequestBody>,
) -> Response {
    match execute_on_device(&state, device_id, body.command, body.cwd, body.timeout_ms).await {
        Ok(response) => Json(response).into_response(),
        Err(ExecDispatchError::BadRequest(message)) => {
            (StatusCode::BAD_REQUEST, message).into_response()
        }
        Err(ExecDispatchError::Offline) => {
            (StatusCode::NOT_FOUND, "device is not online").into_response()
        }
        Err(ExecDispatchError::ConnectionClosed) => {
            (StatusCode::SERVICE_UNAVAILABLE, "device connection closed").into_response()
        }
        Err(ExecDispatchError::ResponseClosed) => {
            (StatusCode::BAD_GATEWAY, "agent response channel closed").into_response()
        }
    }
}

async fn execute_on_device(
    state: &AppState,
    device_id: String,
    command: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<AgentResponse, ExecDispatchError> {
    if command.trim().is_empty() || command.len() > 32 * 1024 {
        return Err(ExecDispatchError::BadRequest(
            "command must be 1..32768 bytes",
        ));
    }
    let timeout_ms = timeout_ms.unwrap_or(30_000).clamp(100, 120_000);
    let tx = {
        let devices = state.devices.read().await;
        devices
            .get(&device_id)
            .map(|d| d.tx.clone())
            .ok_or(ExecDispatchError::Offline)?
    };

    let id = Uuid::new_v4();
    let request = GatewayToAgent::Request(AgentRequest {
        id,
        operation: AgentOperation::Exec {
            command,
            cwd,
            timeout_ms: Some(timeout_ms),
        },
    });
    let (reply_tx, reply_rx) = oneshot::channel();
    state.pending.lock().await.insert(id, reply_tx);

    if tx.send(request).await.is_err() {
        state.pending.lock().await.remove(&id);
        return Err(ExecDispatchError::ConnectionClosed);
    }

    match tokio::time::timeout(Duration::from_millis(timeout_ms + 5_000), reply_rx).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(_)) => Err(ExecDispatchError::ResponseClosed),
        Err(_) => {
            state.pending.lock().await.remove(&id);
            Ok(AgentResponse {
                id,
                result: OperationResult::OutcomeUnknown {
                    error: "gateway timed out waiting for device response".into(),
                },
            })
        }
    }
}

async fn agent_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.max_message_size(DEFAULT_CONTROL_FRAME_LIMIT)
        .on_upgrade(move |socket| handle_agent(socket, state))
}

async fn handle_agent(mut socket: WebSocket, state: AppState) {
    let first = match tokio::time::timeout(Duration::from_secs(5), socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        _ => {
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    let hello = match serde_json::from_str::<AgentToGateway>(first.as_str()) {
        Ok(AgentToGateway::Hello(hello)) => hello,
        _ => {
            warn!("agent sent invalid hello");
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    if hello.protocol != PROTOCOL_VERSION || hello.token.as_str() != state.agent_token.as_ref() {
        warn!(device_id=%hello.device_id, protocol=hello.protocol, "agent authentication/protocol rejected");
        let _ = socket.send(Message::Close(None)).await;
        return;
    }

    let generation = state.next_generation.fetch_add(1, Ordering::Relaxed);
    let (out_tx, mut out_rx) = mpsc::channel::<GatewayToAgent>(64);
    let ack = GatewayToAgent::HelloAck(HelloAck {
        protocol: PROTOCOL_VERSION,
        connection_generation: generation,
        gateway_epoch: state.gateway_epoch,
        control_frame_limit: DEFAULT_CONTROL_FRAME_LIMIT,
    });
    if send_json(&mut socket, &ack).await.is_err() {
        return;
    }

    {
        let mut devices = state.devices.write().await;
        devices.insert(
            hello.device_id.clone(),
            DeviceConnection {
                hello: hello.clone(),
                generation,
                connected_at: SystemTime::now(),
                tx: out_tx,
            },
        );
    }
    info!(device_id=%hello.device_id, generation, platform=%hello.platform, arch=%hello.arch, "agent online");

    loop {
        tokio::select! {
            maybe_out = out_rx.recv() => {
                match maybe_out {
                    Some(message) => {
                        if let Err(err) = send_json(&mut socket, &message).await {
                            warn!(device_id=%hello.device_id, %err, "failed to send to agent");
                            break;
                        }
                    }
                    None => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<AgentToGateway>(text.as_str()) {
                            Ok(AgentToGateway::Response(response)) => {
                                if let Some(waiter) = state.pending.lock().await.remove(&response.id) {
                                    let _ = waiter.send(response);
                                }
                            }
                            Ok(AgentToGateway::Pong) => {}
                            Ok(AgentToGateway::Hello(_)) => warn!(device_id=%hello.device_id, "duplicate hello ignored"),
                            Err(err) => warn!(device_id=%hello.device_id, %err, "invalid agent frame"),
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        warn!(device_id=%hello.device_id, %err, "agent websocket error");
                        break;
                    }
                }
            }
        }
    }

    let mut devices = state.devices.write().await;
    if devices
        .get(&hello.device_id)
        .is_some_and(|d| d.generation == generation)
    {
        devices.remove(&hello.device_id);
        info!(device_id=%hello.device_id, generation, "agent offline");
    } else {
        info!(device_id=%hello.device_id, generation, "stale agent connection retired");
    }
}

async fn send_json(socket: &mut WebSocket, message: &GatewayToAgent) -> anyhow::Result<()> {
    let text = serde_json::to_string(message)?;
    socket.send(Message::Text(text.into())).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::collections::BTreeMap;
    use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};

    const TEST_TOKEN: &str = "developmenttoken123456";

    async fn spawn_test_gateway() -> (AppState, SocketAddr, tokio::task::JoinHandle<()>) {
        let state = AppState::new(TEST_TOKEN.into());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = build_app(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (state, addr, task)
    }

    async fn connect_agent(
        addr: SocketAddr,
        device_id: &str,
    ) -> (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        u64,
    ) {
        let (mut socket, _) = connect_async(format!("ws://{addr}/agent")).await.unwrap();
        let hello = AgentToGateway::Hello(DeviceHello {
            protocol: PROTOCOL_VERSION,
            token: TEST_TOKEN.into(),
            device_id: device_id.into(),
            device_name: device_id.into(),
            agent_version: "test".into(),
            platform: "linux".into(),
            arch: "aarch64".into(),
            boot_id: "boot-test".into(),
            agent_instance_id: Uuid::new_v4(),
            capabilities: vec!["exec".into()],
            metadata: BTreeMap::new(),
        });
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&hello).unwrap().into(),
            ))
            .await
            .unwrap();
        let frame = socket.next().await.unwrap().unwrap();
        let ClientMessage::Text(text) = frame else {
            panic!("expected hello ack text frame");
        };
        let ack: GatewayToAgent = serde_json::from_str(text.as_str()).unwrap();
        let GatewayToAgent::HelloAck(ack) = ack else {
            panic!("expected hello ack");
        };
        (socket, ack.connection_generation)
    }

    async fn wait_registered(state: &AppState, device_id: &str) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if state.devices.read().await.contains_key(device_id) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn handshake_registers_device_and_exec_round_trips() {
        let (state, addr, server) = spawn_test_gateway().await;
        let (socket, generation) = connect_agent(addr, "test-device").await;
        wait_registered(&state, "test-device").await;
        assert_eq!(generation, 1);

        let (mut sink, mut stream) = socket.split();
        let responder = tokio::spawn(async move {
            while let Some(Ok(ClientMessage::Text(text))) = stream.next().await {
                let message: GatewayToAgent = serde_json::from_str(text.as_str()).unwrap();
                if let GatewayToAgent::Request(request) = message {
                    let output = ExecOutput {
                        exit_code: Some(0),
                        stdout: "integration-ok\n".into(),
                        stderr: String::new(),
                        truncated: false,
                    };
                    let response = AgentToGateway::Response(AgentResponse {
                        id: request.id,
                        result: OperationResult::Ok {
                            body: serde_json::to_value(output).unwrap(),
                        },
                    });
                    sink.send(ClientMessage::Text(
                        serde_json::to_string(&response).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                }
            }
        });

        let response = execute_on_device(
            &state,
            "test-device".into(),
            "echo ignored-by-simulator".into(),
            None,
            Some(2_000),
        )
        .await
        .unwrap();
        let OperationResult::Ok { body } = response.result else {
            panic!("expected ok response");
        };
        let output: ExecOutput = serde_json::from_value(body).unwrap();
        assert_eq!(output.stdout, "integration-ok\n");

        responder.abort();
        server.abort();
    }

    #[tokio::test]
    async fn newer_connection_supersedes_generation() {
        let (state, addr, server) = spawn_test_gateway().await;
        let (first, first_generation) = connect_agent(addr, "same-device").await;
        wait_registered(&state, "same-device").await;
        let (second, second_generation) = connect_agent(addr, "same-device").await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let generation = state
                    .devices
                    .read()
                    .await
                    .get("same-device")
                    .map(|d| d.generation);
                if generation == Some(second_generation) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        assert!(second_generation > first_generation);
        drop(first);
        drop(second);
        server.abort();
    }
}
