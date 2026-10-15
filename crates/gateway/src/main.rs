mod store;

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, VerifyingKey};
use infra_protocol::{
    AgentOperation, AgentRequest, AgentResponse, AgentToGateway, AuthChallenge,
    DEFAULT_CONTROL_FRAME_LIMIT, DeviceAuthStart, DeviceHello, ExecOutput, FileAction, FileRequest,
    GatewayToAgent, HelloAck, JobAction, JobRequest, LegacyDeviceHello, OperationResult,
    PROTOCOL_VERSION, ProcessAction, ProcessRequest, ReconcileSnapshot, SearchMode, SearchRequest,
    SignedDeviceHello, TerminalAction, TerminalRequest, TransferAction, TransferRequest,
    signed_auth_message,
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
    convert::Infallible,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use store::{DeviceRecord, Store, StoreError, now_ms};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    legacy_agent_token: Option<Arc<str>>,
    admin_token: Option<Arc<str>>,
    gateway_epoch: Uuid,
    next_generation: Arc<AtomicU64>,
    devices: Arc<RwLock<HashMap<String, DeviceConnection>>>,
    pending: Arc<Mutex<HashMap<Uuid, oneshot::Sender<AgentResponse>>>>,
    events: broadcast::Sender<DashboardEvent>,
    store: Store,
}

impl AppState {
    fn new(store: Store, legacy_agent_token: Option<String>, admin_token: Option<String>) -> Self {
        let (events, _) = broadcast::channel(128);
        Self {
            legacy_agent_token: legacy_agent_token.map(Arc::from),
            admin_token: admin_token.map(Arc::from),
            gateway_epoch: Uuid::new_v4(),
            next_generation: Arc::new(AtomicU64::new(1)),
            devices: Arc::new(RwLock::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            events,
            store,
        }
    }

    fn emit(&self, kind: &'static str, device_id: Option<String>) {
        let _ = self.events.send(DashboardEvent {
            kind,
            device_id,
            at_unix_ms: now_ms(),
        });
    }
}

#[derive(Debug, Clone, Serialize)]
struct DashboardEvent {
    kind: &'static str,
    device_id: Option<String>,
    at_unix_ms: u64,
}

#[derive(Clone)]
struct DeviceConnection {
    hello: DeviceHello,
    generation: u64,
    tx: mpsc::Sender<GatewayToAgent>,
    reconcile: ReconcileSnapshot,
}

#[derive(Debug, Serialize)]
struct Health<'a> {
    status: &'a str,
    service: &'a str,
    protocol: u16,
    gateway_epoch: Uuid,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
struct McpDeviceView {
    device_id: String,
    device_name: String,
    agent_version: String,
    platform: String,
    arch: String,
    capabilities: Vec<String>,
    online: bool,
    revoked: bool,
    last_seen_unix_ms: u64,
    connection_generation: Option<u64>,
    active_jobs: usize,
    active_terminals: usize,
    active_transfers: usize,
}

#[derive(Debug, Clone, Serialize)]
struct DashboardSession {
    id: Uuid,
    kind: &'static str,
    status: String,
    boot_id: String,
    started_at_unix_ms: u64,
    device_id: String,
    device_name: String,
    device_online: bool,
}

#[derive(Debug, Deserialize)]
struct CreateEnrollmentInput {
    #[serde(default)]
    ttl_seconds: Option<u64>,
}
#[derive(Debug, Serialize)]
struct CreateEnrollmentOutput {
    enrollment_id: String,
    code: String,
    expires_at_unix_ms: u64,
}
#[derive(Debug, Deserialize)]
struct EnrollInput {
    code: String,
    request_id: String,
    device_name: String,
    public_key: String,
    platform: String,
    arch: String,
    agent_version: String,
}
#[derive(Debug, Serialize)]
struct EnrollOutput {
    device_id: String,
    device_name: String,
}

#[derive(Debug)]
enum DispatchError {
    Offline,
    ConnectionClosed,
    ResponseClosed,
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

#[derive(Debug, Deserialize, JsonSchema)]
struct McpFilesInput {
    device: String,
    action: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    destination: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    length: Option<usize>,
    #[serde(default)]
    recursive: Option<bool>,
    #[serde(default)]
    create_parents: Option<bool>,
    #[serde(default)]
    old: Option<String>,
    #[serde(default)]
    new: Option<String>,
    #[serde(default)]
    expected_replacements: Option<usize>,
}
#[derive(Debug, Deserialize, JsonSchema)]
struct McpSearchInput {
    device: String,
    root: String,
    query: String,
    mode: String,
    #[serde(default)]
    literal: Option<bool>,
    #[serde(default)]
    ignore_case: Option<bool>,
    #[serde(default)]
    include_hidden: Option<bool>,
    #[serde(default)]
    file_pattern: Option<String>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    context_lines: Option<usize>,
}
#[derive(Debug, Deserialize, JsonSchema)]
struct McpProcessesInput {
    device: String,
    action: String,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    force: Option<bool>,
}
#[derive(Debug, Deserialize, JsonSchema)]
struct McpJobsInput {
    device: String,
    action: String,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    job_id: Option<String>,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    length: Option<usize>,
}
#[derive(Debug, Deserialize, JsonSchema)]
struct McpTerminalInput {
    device: String,
    action: String,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    rows: Option<u16>,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    length: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpTransferInput {
    device: String,
    action: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    overwrite: Option<bool>,
    #[serde(default)]
    create_parents: Option<bool>,
    #[serde(default)]
    transfer_id: Option<String>,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    data_base64: Option<String>,
    #[serde(default)]
    length: Option<usize>,
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
        description = "List enrolled Infra devices, online state, capabilities and active persistent work."
    )]
    async fn devices(&self) -> Result<McpJson<Vec<McpDeviceView>>, String> {
        Ok(McpJson(
            device_views(&self.state).await.map_err(|e| e.to_string())?,
        ))
    }

    #[tool(
        name = "exec",
        description = "Run a bounded one-shot command on an online Infra device as the agent user."
    )]
    async fn exec(
        &self,
        Parameters(input): Parameters<McpExecInput>,
    ) -> Result<McpJson<McpExecOutput>, String> {
        let device_id = input.device.clone();
        let response = dispatch_to_device(
            &self.state,
            input.device,
            AgentOperation::Exec {
                command: input.command,
                cwd: input.cwd,
                timeout_ms: input.timeout_ms,
            },
            input.timeout_ms.unwrap_or(30_000).clamp(100, 120_000) + 5_000,
        )
        .await
        .map_err(dispatch_error)?;
        record_operation(&self.state, &device_id, "exec", &response);
        Ok(McpJson(exec_output(response)))
    }

    #[tool(
        name = "files",
        description = "Read or mutate files on an Infra device. Actions: read, write, append, exact_edit, hash, list, stat, mkdir, copy, move, trash, delete_permanent."
    )]
    async fn files(
        &self,
        Parameters(input): Parameters<McpFilesInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let action = match input.action.as_str() {
            "read" => FileAction::Read {
                path: required(input.path, "path")?,
                offset: input.offset,
                length: input.length,
            },
            "write" => FileAction::Write {
                path: required(input.path, "path")?,
                content: required(input.content, "content")?,
                create_parents: input.create_parents.unwrap_or(false),
            },
            "append" => FileAction::Append {
                path: required(input.path, "path")?,
                content: required(input.content, "content")?,
                create_parents: input.create_parents.unwrap_or(false),
            },
            "exact_edit" => FileAction::ExactEdit {
                path: required(input.path, "path")?,
                old: required(input.old, "old")?,
                new: required(input.new, "new")?,
                expected_replacements: input.expected_replacements,
            },
            "hash" => FileAction::Hash {
                path: required(input.path, "path")?,
            },
            "list" => FileAction::List {
                path: required(input.path, "path")?,
            },
            "stat" => FileAction::Stat {
                path: required(input.path, "path")?,
            },
            "mkdir" => FileAction::Mkdir {
                path: required(input.path, "path")?,
            },
            "copy" => FileAction::Copy {
                source: required(input.path, "path")?,
                destination: required(input.destination, "destination")?,
                recursive: input.recursive.unwrap_or(false),
            },
            "move" => FileAction::Move {
                source: required(input.path, "path")?,
                destination: required(input.destination, "destination")?,
            },
            "trash" => FileAction::Trash {
                path: required(input.path, "path")?,
            },
            "delete_permanent" => FileAction::DeletePermanent {
                path: required(input.path, "path")?,
                recursive: input.recursive.unwrap_or(false),
            },
            _ => return Err("unknown files action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Files(FileRequest { action }),
            65_000,
        )
        .await
    }

    #[tool(
        name = "search",
        description = "Search filenames or text content efficiently on an Infra device. mode: files or content."
    )]
    async fn search(
        &self,
        Parameters(input): Parameters<McpSearchInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let mode = match input.mode.as_str() {
            "files" => SearchMode::Files,
            "content" => SearchMode::Content,
            _ => return Err("search mode must be files or content".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Search(SearchRequest {
                root: input.root,
                query: input.query,
                mode,
                literal: input.literal.unwrap_or(false),
                ignore_case: input.ignore_case.unwrap_or(true),
                include_hidden: input.include_hidden.unwrap_or(false),
                file_pattern: input.file_pattern,
                max_results: input.max_results,
                context_lines: input.context_lines,
            }),
            65_000,
        )
        .await
    }

    #[tool(
        name = "processes",
        description = "List/find or terminate processes on an Infra device. Actions: list, terminate."
    )]
    async fn processes(
        &self,
        Parameters(input): Parameters<McpProcessesInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let action = match input.action.as_str() {
            "list" => ProcessAction::List { query: input.query },
            "terminate" => ProcessAction::Terminate {
                pid: input.pid.ok_or("pid is required")?,
                force: input.force.unwrap_or(false),
            },
            _ => return Err("unknown processes action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Processes(ProcessRequest { action }),
            30_000,
        )
        .await
    }

    #[tool(
        name = "jobs",
        description = "Manage durable background command jobs. Actions: start, list, read, cancel."
    )]
    async fn jobs(
        &self,
        Parameters(input): Parameters<McpJobsInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let action = match input.action.as_str() {
            "start" => JobAction::Start {
                command: required(input.command, "command")?,
                cwd: input.cwd,
                timeout_ms: input.timeout_ms,
            },
            "list" => JobAction::List,
            "read" => JobAction::Read {
                job_id: parse_uuid(input.job_id, "job_id")?,
                offset: input.offset,
                length: input.length,
            },
            "cancel" => JobAction::Cancel {
                job_id: parse_uuid(input.job_id, "job_id")?,
            },
            _ => return Err("unknown jobs action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Jobs(JobRequest { action }),
            65_000,
        )
        .await
    }

    #[tool(
        name = "terminal",
        description = "Manage persistent PTY sessions independent of the MCP request. Actions: open, list, read, write, resize, close."
    )]
    async fn terminal(
        &self,
        Parameters(input): Parameters<McpTerminalInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let action = match input.action.as_str() {
            "open" => TerminalAction::Open {
                command: required(input.command, "command")?,
                cwd: input.cwd,
                rows: input.rows,
                cols: input.cols,
            },
            "list" => TerminalAction::List,
            "read" => TerminalAction::Read {
                session_id: parse_uuid(input.session_id, "session_id")?,
                offset: input.offset,
                length: input.length,
            },
            "write" => TerminalAction::Write {
                session_id: parse_uuid(input.session_id, "session_id")?,
                data: required(input.data, "data")?,
            },
            "resize" => TerminalAction::Resize {
                session_id: parse_uuid(input.session_id, "session_id")?,
                rows: input.rows.ok_or("rows is required")?,
                cols: input.cols.ok_or("cols is required")?,
            },
            "close" => TerminalAction::Close {
                session_id: parse_uuid(input.session_id, "session_id")?,
            },
            _ => return Err("unknown terminal action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Terminal(TerminalRequest { action }),
            65_000,
        )
        .await
    }

    #[tool(
        name = "transfer",
        description = "Resumable binary upload/download on an Infra device. Actions: upload_begin, upload_chunk, upload_commit, download_begin, download_chunk, status, list, cancel."
    )]
    async fn transfer(
        &self,
        Parameters(input): Parameters<McpTransferInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let action = match input.action.as_str() {
            "upload_begin" => TransferAction::UploadBegin {
                path: required(input.path, "path")?,
                size: input.size.ok_or("size is required")?,
                sha256: input.sha256,
                overwrite: input.overwrite.unwrap_or(false),
                create_parents: input.create_parents.unwrap_or(false),
            },
            "upload_chunk" => TransferAction::UploadChunk {
                transfer_id: parse_uuid(input.transfer_id, "transfer_id")?,
                offset: input.offset.ok_or("offset is required")?,
                data_base64: required(input.data_base64, "data_base64")?,
            },
            "upload_commit" => TransferAction::UploadCommit {
                transfer_id: parse_uuid(input.transfer_id, "transfer_id")?,
            },
            "download_begin" => TransferAction::DownloadBegin {
                path: required(input.path, "path")?,
            },
            "download_chunk" => TransferAction::DownloadChunk {
                transfer_id: parse_uuid(input.transfer_id, "transfer_id")?,
                offset: input.offset.ok_or("offset is required")?,
                length: input.length,
            },
            "status" => TransferAction::Status {
                transfer_id: parse_uuid(input.transfer_id, "transfer_id")?,
            },
            "list" => TransferAction::List,
            "cancel" => TransferAction::Cancel {
                transfer_id: parse_uuid(input.transfer_id, "transfer_id")?,
            },
            _ => return Err("unknown transfer action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Transfer(TransferRequest { action }),
            300_000,
        )
        .await
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
    let bind: SocketAddr = std::env::var("INFRA_GATEWAY_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8788".into())
        .parse()
        .context("invalid INFRA_GATEWAY_BIND")?;
    if !bind.ip().is_loopback()
        && std::env::var("INFRA_ALLOW_UNAUTHENTICATED_PUBLIC").as_deref() != Ok("1")
    {
        anyhow::bail!(
            "refusing non-loopback bind until production OAuth exists; set INFRA_ALLOW_UNAUTHENTICATED_PUBLIC=1 only for an explicitly protected development deployment"
        );
    }
    let db_path = PathBuf::from(
        std::env::var("INFRA_DB_PATH").unwrap_or_else(|_| "./data/infra-v1.db".into()),
    );
    let store = Store::open(db_path)?;
    let legacy = std::env::var("INFRA_LEGACY_AGENT_TOKEN")
        .ok()
        .or_else(|| std::env::var("INFRA_AGENT_TOKEN").ok());
    let admin = std::env::var("INFRA_ADMIN_TOKEN").ok();
    let state = AppState::new(store, legacy, admin);
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
        .route("/", get(dashboard_index))
        .route("/app.js", get(dashboard_js))
        .route("/styles.css", get(dashboard_css))
        .route("/downloads/install.sh", get(download_install_sh))
        .route("/downloads/install.ps1", get(download_install_ps1))
        .route("/healthz", get(health))
        .route("/api/devices", get(list_devices_http))
        .route("/api/sessions", get(list_sessions_http))
        .route("/api/activity", get(list_activity_http))
        .route("/api/events", get(dashboard_events))
        .route("/api/enrollments", post(create_enrollment))
        .route("/api/devices/{device_id}/revoke", post(revoke_device))
        .route("/enroll", post(enroll))
        .route("/agent", get(agent_upgrade))
        .with_state(state)
}

const DASHBOARD_HTML: &str = include_str!("../../../apps/dashboard/index.html");
const DASHBOARD_JS: &str = include_str!("../../../apps/dashboard/app.js");
const DASHBOARD_CSS: &str = include_str!("../../../apps/dashboard/styles.css");
const INSTALL_SH: &str = include_str!("../../../scripts/install.sh");
const INSTALL_PS1: &str = include_str!("../../../scripts/install.ps1");

async fn dashboard_index() -> Response {
    static_response(DASHBOARD_HTML, "text/html; charset=utf-8")
}
async fn dashboard_js() -> Response {
    static_response(DASHBOARD_JS, "text/javascript; charset=utf-8")
}
async fn dashboard_css() -> Response {
    static_response(DASHBOARD_CSS, "text/css; charset=utf-8")
}
async fn download_install_sh() -> Response {
    static_response(INSTALL_SH, "text/x-shellscript; charset=utf-8")
}
async fn download_install_ps1() -> Response {
    static_response(INSTALL_PS1, "text/plain; charset=utf-8")
}

fn static_response(body: &'static str, content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
            ),
        ],
        body,
    )
        .into_response()
}

async fn dashboard_events(
    State(state): State<AppState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.events.subscribe();
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let payload = serde_json::to_string(&event)
                        .unwrap_or_else(|_| "{\"kind\":\"refresh\"}".into());
                    return Some((Ok(Event::default().data(payload)), receiver));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

async fn list_sessions_http(State(state): State<AppState>) -> Response {
    match dashboard_sessions(&state).await {
        Ok(sessions) => Json(sessions).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn list_activity_http(State(state): State<AppState>) -> Response {
    match state.store.list_activity(100) {
        Ok(activity) => Json(activity).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn dashboard_sessions(state: &AppState) -> Result<Vec<DashboardSession>, StoreError> {
    let persisted = state.store.list_devices()?;
    let online = state.devices.read().await;
    let mut sessions = Vec::new();
    for record in persisted {
        let live = online.get(&record.device_id);
        let reconcile = live
            .map(|device| &device.reconcile)
            .or(record.reconcile.as_ref());
        let Some(reconcile) = reconcile else {
            continue;
        };
        for item in &reconcile.items {
            sessions.push(DashboardSession {
                id: item.id,
                kind: match item.kind {
                    infra_protocol::ReconcileKind::Job => "job",
                    infra_protocol::ReconcileKind::Terminal => "terminal",
                    infra_protocol::ReconcileKind::Transfer => "transfer",
                },
                status: item.status.clone(),
                boot_id: item.boot_id.clone(),
                started_at_unix_ms: item.started_at_unix_ms,
                device_id: record.device_id.clone(),
                device_name: record.device_name.clone(),
                device_online: live.is_some(),
            });
        }
    }
    sessions.sort_by_key(|session| std::cmp::Reverse(session.started_at_unix_ms));
    Ok(sessions)
}

async fn health(State(state): State<AppState>) -> Json<Health<'static>> {
    Json(Health {
        status: "ok",
        service: "infra-gateway",
        protocol: PROTOCOL_VERSION,
        gateway_epoch: state.gateway_epoch,
    })
}
async fn list_devices_http(State(state): State<AppState>) -> Response {
    match device_views(&state).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn create_enrollment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateEnrollmentInput>,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "admin authorization required").into_response();
    }
    let ttl_ms = input.ttl_seconds.unwrap_or(300).clamp(30, 900) * 1000;
    match state.store.create_enrollment(ttl_ms) {
        Ok(created) => {
            let _ = state.store.record_activity(
                None,
                "enrollment.create",
                "ok",
                Some(&created.enrollment_id),
            );
            state.emit("enrollment_created", None);
            Json(CreateEnrollmentOutput {
                enrollment_id: created.enrollment_id,
                code: created.code,
                expires_at_unix_ms: created.expires_at_unix_ms,
            })
            .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn enroll(State(state): State<AppState>, Json(input): Json<EnrollInput>) -> Response {
    if Uuid::parse_str(&input.request_id).is_err()
        || input.device_name.trim().is_empty()
        || input.device_name.len() > 120
    {
        return (StatusCode::BAD_REQUEST, "invalid enrollment request").into_response();
    }
    let key = match URL_SAFE_NO_PAD
        .decode(&input.public_key)
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
    {
        Some(k) => k,
        None => return (StatusCode::BAD_REQUEST, "invalid public key").into_response(),
    };
    match state.store.consume_enrollment(
        &input.code,
        &input.request_id,
        input.device_name.trim(),
        &key,
        &input.platform,
        &input.arch,
        &input.agent_version,
    ) {
        Ok(result) => {
            let device_id = result.device_id.clone();
            let _ = state.store.record_activity(
                Some(&device_id),
                "enrollment.consume",
                "ok",
                Some(&input.request_id),
            );
            state.emit("device_enrolled", Some(device_id));
            Json(EnrollOutput {
                device_id: result.device_id,
                device_name: result.device_name,
            })
            .into_response()
        }
        Err(StoreError::EnrollmentInvalid) => {
            (StatusCode::UNAUTHORIZED, "invalid enrollment code").into_response()
        }
        Err(StoreError::EnrollmentExpired) => {
            (StatusCode::GONE, "enrollment code expired").into_response()
        }
        Err(StoreError::EnrollmentConsumed) => {
            (StatusCode::CONFLICT, "enrollment code already used").into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn revoke_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "admin authorization required").into_response();
    }
    match state.store.revoke(&device_id) {
        Ok(true) => {
            state.devices.write().await.remove(&device_id);
            let _ = state
                .store
                .record_activity(Some(&device_id), "device.revoke", "ok", None);
            state.emit("device_revoked", Some(device_id.clone()));
            Json(serde_json::json!({"ok":true,"device_id":device_id})).into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "device not found").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn admin_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(expected) = &state.admin_token else {
        return true;
    };
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.strip_prefix("Bearer ") == Some(expected.as_ref()))
}

async fn device_views(state: &AppState) -> Result<Vec<McpDeviceView>, StoreError> {
    let persisted = state.store.list_devices()?;
    let online = state.devices.read().await;
    Ok(persisted
        .into_iter()
        .map(|record| {
            let device_id = record.device_id.clone();
            device_view(record, online.get(&device_id))
        })
        .collect())
}
fn device_view(record: DeviceRecord, live: Option<&DeviceConnection>) -> McpDeviceView {
    let reconcile = live.map(|d| &d.reconcile).or(record.reconcile.as_ref());
    let active_jobs = reconcile
        .map(|r| {
            r.items
                .iter()
                .filter(|i| matches!(i.kind, infra_protocol::ReconcileKind::Job))
                .count()
        })
        .unwrap_or(0);
    let active_terminals = reconcile
        .map(|r| {
            r.items
                .iter()
                .filter(|i| matches!(i.kind, infra_protocol::ReconcileKind::Terminal))
                .count()
        })
        .unwrap_or(0);
    let active_transfers = reconcile
        .map(|r| {
            r.items
                .iter()
                .filter(|i| matches!(i.kind, infra_protocol::ReconcileKind::Transfer))
                .count()
        })
        .unwrap_or(0);
    McpDeviceView {
        device_id: record.device_id,
        device_name: record.device_name,
        agent_version: live
            .map(|d| d.hello.agent_version.clone())
            .unwrap_or(record.agent_version),
        platform: record.platform,
        arch: record.arch,
        capabilities: live
            .map(|d| d.hello.capabilities.clone())
            .unwrap_or(record.capabilities),
        online: live.is_some(),
        revoked: record.revoked,
        last_seen_unix_ms: record.last_seen_unix_ms,
        connection_generation: live.map(|d| d.generation),
        active_jobs,
        active_terminals,
        active_transfers,
    }
}

async fn tool_operation(
    state: &AppState,
    device: String,
    operation: AgentOperation,
    timeout_ms: u64,
) -> Result<McpJson<serde_json::Value>, String> {
    let action = operation_label(&operation);
    let response = dispatch_to_device(state, device.clone(), operation, timeout_ms)
        .await
        .map_err(dispatch_error)?;
    record_operation(state, &device, action, &response);
    Ok(McpJson(response_json(response)))
}

fn operation_label(operation: &AgentOperation) -> &'static str {
    match operation {
        AgentOperation::Status => "status",
        AgentOperation::Exec { .. } => "exec",
        AgentOperation::Files(_) => "files",
        AgentOperation::Search(_) => "search",
        AgentOperation::Processes(_) => "processes",
        AgentOperation::Jobs(_) => "jobs",
        AgentOperation::Terminal(_) => "terminal",
        AgentOperation::Transfer(_) => "transfer",
    }
}

fn outcome_label(result: &OperationResult) -> &'static str {
    match result {
        OperationResult::Ok { .. } => "ok",
        OperationResult::Failed { .. } => "failed",
        OperationResult::Canceled { .. } => "canceled",
        OperationResult::DeadlineExceeded => "deadline_exceeded",
        OperationResult::OutcomeUnknown { .. } => "outcome_unknown",
        OperationResult::Unsupported { .. } => "unsupported",
        OperationResult::Unauthorized => "unauthorized",
    }
}

fn record_operation(state: &AppState, device_id: &str, action: &str, response: &AgentResponse) {
    let _ = state.store.record_activity(
        Some(device_id),
        action,
        outcome_label(&response.result),
        Some(&response.id.to_string()),
    );
    state.emit("activity", Some(device_id.to_string()));
}
fn response_json(response: AgentResponse) -> serde_json::Value {
    match response.result {
        OperationResult::Ok { body } => {
            serde_json::json!({"request_id":response.id,"status":"ok","body":body})
        }
        OperationResult::Failed { error } => {
            serde_json::json!({"request_id":response.id,"status":"failed","error":error})
        }
        OperationResult::Canceled { error } => {
            serde_json::json!({"request_id":response.id,"status":"canceled","error":error})
        }
        OperationResult::DeadlineExceeded => {
            serde_json::json!({"request_id":response.id,"status":"deadline_exceeded"})
        }
        OperationResult::OutcomeUnknown { error } => {
            serde_json::json!({"request_id":response.id,"status":"outcome_unknown","error":error})
        }
        OperationResult::Unsupported { operation } => {
            serde_json::json!({"request_id":response.id,"status":"unsupported","error":operation})
        }
        OperationResult::Unauthorized => {
            serde_json::json!({"request_id":response.id,"status":"unauthorized"})
        }
    }
}
fn exec_output(response: AgentResponse) -> McpExecOutput {
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
                Ok(v) => {
                    out.exit_code = v.exit_code;
                    out.stdout = Some(v.stdout);
                    out.stderr = Some(v.stderr);
                    out.truncated = Some(v.truncated)
                }
                Err(e) => out.error = Some(format!("invalid exec response: {e}")),
            }
        }
        OperationResult::Failed { error } => {
            out.status = "failed".into();
            out.error = Some(error)
        }
        OperationResult::Canceled { error } => {
            out.status = "canceled".into();
            out.error = error
        }
        OperationResult::DeadlineExceeded => out.status = "deadline_exceeded".into(),
        OperationResult::OutcomeUnknown { error } => {
            out.status = "outcome_unknown".into();
            out.error = Some(error)
        }
        OperationResult::Unsupported { operation } => {
            out.status = "unsupported".into();
            out.error = Some(operation)
        }
        OperationResult::Unauthorized => out.status = "unauthorized".into(),
    };
    out
}
fn required<T>(value: Option<T>, name: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("{name} is required"))
}
fn parse_uuid(value: Option<String>, name: &str) -> Result<Uuid, String> {
    Uuid::parse_str(&required(value, name)?).map_err(|_| format!("{name} must be a UUID"))
}
fn dispatch_error(error: DispatchError) -> String {
    match error {
        DispatchError::Offline => "device is not online".into(),
        DispatchError::ConnectionClosed => "device connection closed".into(),
        DispatchError::ResponseClosed => "agent response channel closed".into(),
    }
}

async fn dispatch_to_device(
    state: &AppState,
    device_id: String,
    operation: AgentOperation,
    timeout_ms: u64,
) -> Result<AgentResponse, DispatchError> {
    let tx = {
        let devices = state.devices.read().await;
        devices
            .get(&device_id)
            .map(|d| d.tx.clone())
            .ok_or(DispatchError::Offline)?
    };
    let id = Uuid::new_v4();
    let request = GatewayToAgent::Request(AgentRequest { id, operation });
    let (reply_tx, reply_rx) = oneshot::channel();
    state.pending.lock().await.insert(id, reply_tx);
    if tx.send(request).await.is_err() {
        state.pending.lock().await.remove(&id);
        return Err(DispatchError::ConnectionClosed);
    }
    match tokio::time::timeout(Duration::from_millis(timeout_ms), reply_rx).await {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(_)) => Err(DispatchError::ResponseClosed),
        Err(_) => {
            state.pending.lock().await.remove(&id);
            Ok(AgentResponse {
                id,
                result: OperationResult::OutcomeUnknown {
                    error:
                        "gateway timed out waiting for device response; operation was not replayed"
                            .into(),
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
    let auth = match authenticate_agent(&mut socket, &state).await {
        Ok(v) => v,
        Err(e) => {
            warn!(error=%e,"agent authentication rejected");
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    let hello = auth.hello;
    let generation = state.next_generation.fetch_add(1, Ordering::Relaxed);
    let (out_tx, mut out_rx) = mpsc::channel::<GatewayToAgent>(64);
    let ack = GatewayToAgent::HelloAck(HelloAck {
        protocol: PROTOCOL_VERSION,
        connection_generation: generation,
        gateway_epoch: state.gateway_epoch,
        control_frame_limit: DEFAULT_CONTROL_FRAME_LIMIT,
        reconcile_requested: true,
    });
    if send_json(&mut socket, &ack).await.is_err() {
        return;
    }
    if let Err(e) = state
        .store
        .record_connect(&hello, auth.public_key.as_deref())
    {
        warn!(device_id=%hello.device_id,error=%e,"failed to persist device connect");
    }
    {
        let mut devices = state.devices.write().await;
        devices.insert(
            hello.device_id.clone(),
            DeviceConnection {
                hello: hello.clone(),
                generation,
                tx: out_tx,
                reconcile: ReconcileSnapshot::default(),
            },
        );
    }
    let _ = state
        .store
        .record_activity(Some(&hello.device_id), "device.connect", "ok", None);
    state.emit("device_online", Some(hello.device_id.clone()));
    info!(device_id=%hello.device_id,generation,platform=%hello.platform,arch=%hello.arch,"agent online");
    loop {
        tokio::select! {
            maybe_out=out_rx.recv()=>match maybe_out{Some(message)=>if let Err(e)=send_json(&mut socket,&message).await{warn!(device_id=%hello.device_id,error=%e,"failed to send to agent");break},None=>break},
            incoming=socket.recv()=>match incoming{
                Some(Ok(Message::Text(text)))=>match serde_json::from_str::<AgentToGateway>(text.as_str()){
                    Ok(AgentToGateway::Response(response))=>if let Some(waiter)=state.pending.lock().await.remove(&response.id){let _=waiter.send(response);},
                    Ok(AgentToGateway::Reconcile(snapshot)) => {
                        if let Some(device) = state.devices.write().await.get_mut(&hello.device_id)
                            && device.generation == generation
                        {
                            device.reconcile = snapshot.clone();
                        }
                        let _ = state.store.record_reconcile(&hello.device_id, &snapshot);
                        state.emit("sessions_changed", Some(hello.device_id.clone()));
                    },
                    Ok(AgentToGateway::Pong)=>{}, Ok(_)=>warn!(device_id=%hello.device_id,"unexpected post-auth agent frame"), Err(e)=>warn!(device_id=%hello.device_id,error=%e,"invalid agent frame")},
                Some(Ok(Message::Ping(payload)))=>if socket.send(Message::Pong(payload)).await.is_err(){break},
                Some(Ok(Message::Close(_)))|None=>break,Some(Ok(_))=>{},Some(Err(e))=>{warn!(device_id=%hello.device_id,error=%e,"agent websocket error");break}
            }
        }
    }
    let mut devices = state.devices.write().await;
    if devices
        .get(&hello.device_id)
        .is_some_and(|d| d.generation == generation)
    {
        devices.remove(&hello.device_id);
        let _ = state.store.record_disconnect(&hello.device_id);
        let _ =
            state
                .store
                .record_activity(Some(&hello.device_id), "device.disconnect", "ok", None);
        state.emit("device_offline", Some(hello.device_id.clone()));
        info!(device_id=%hello.device_id,generation,"agent offline")
    } else {
        info!(device_id=%hello.device_id,generation,"stale agent connection retired")
    }
}

struct AuthenticatedHello {
    hello: DeviceHello,
    public_key: Option<Vec<u8>>,
}
async fn authenticate_agent(
    socket: &mut WebSocket,
    state: &AppState,
) -> Result<AuthenticatedHello, String> {
    let first = recv_agent(socket, Duration::from_secs(5)).await?;
    match first {
        AgentToGateway::AuthStart(start) => authenticate_signed(socket, state, start).await,
        AgentToGateway::LegacyHello(LegacyDeviceHello { hello, token }) => {
            let Some(expected) = &state.legacy_agent_token else {
                return Err("legacy authentication disabled".into());
            };
            if token.as_str() != expected.as_ref() {
                return Err("invalid legacy credential".into());
            }
            if hello.protocol != PROTOCOL_VERSION {
                return Err("protocol mismatch".into());
            }
            Ok(AuthenticatedHello {
                hello,
                public_key: None,
            })
        }
        _ => Err("expected auth_start or legacy_hello".into()),
    }
}
async fn authenticate_signed(
    socket: &mut WebSocket,
    state: &AppState,
    start: DeviceAuthStart,
) -> Result<AuthenticatedHello, String> {
    if start.protocol != PROTOCOL_VERSION {
        return Err("protocol mismatch".into());
    }
    let record = state
        .store
        .get_device(&start.device_id)
        .map_err(|e| e.to_string())?
        .ok_or("unknown device")?;
    if record.revoked {
        return Err("device revoked".into());
    }
    let public_key = record
        .public_key
        .ok_or("device has no enrolled public key")?;
    let mut nonce_bytes = [0_u8; 32];
    getrandom::fill(&mut nonce_bytes).map_err(|_| "random source unavailable")?;
    let nonce = URL_SAFE_NO_PAD.encode(nonce_bytes);
    let expires = now_ms() + 5_000;
    send_json(
        socket,
        &GatewayToAgent::AuthChallenge(AuthChallenge {
            nonce: nonce.clone(),
            gateway_epoch: state.gateway_epoch,
            expires_at_unix_ms: expires,
        }),
    )
    .await
    .map_err(|e| e.to_string())?;
    let AgentToGateway::SignedHello(SignedDeviceHello { hello, signature }) =
        recv_agent(socket, Duration::from_secs(5)).await?
    else {
        return Err("expected signed_hello".into());
    };
    if now_ms() > expires
        || hello.protocol != PROTOCOL_VERSION
        || hello.device_id != start.device_id
        || hello.agent_instance_id != start.agent_instance_id
    {
        return Err("signed hello does not match authentication start".into());
    }
    let key_bytes = <[u8; 32]>::try_from(public_key.as_slice())
        .map_err(|_| "stored public key has invalid length")?;
    let verifying =
        VerifyingKey::from_bytes(&key_bytes).map_err(|_| "stored public key invalid")?;
    let sig_bytes = <[u8; 64]>::try_from(
        URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| "signature encoding invalid")?,
    )
    .map_err(|_| "signature length invalid")?;
    let sig = Signature::from_bytes(&sig_bytes);
    verifying
        .verify_strict(
            &signed_auth_message(&nonce, state.gateway_epoch, &hello),
            &sig,
        )
        .map_err(|_| "signature verification failed")?;
    Ok(AuthenticatedHello {
        hello,
        public_key: Some(public_key),
    })
}
async fn recv_agent(socket: &mut WebSocket, timeout: Duration) -> Result<AgentToGateway, String> {
    match tokio::time::timeout(timeout, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            serde_json::from_str(text.as_str()).map_err(|e| e.to_string())
        }
        Ok(Some(Err(e))) => Err(e.to_string()),
        Ok(Some(Ok(_))) => Err("unexpected websocket frame".into()),
        Ok(None) => Err("connection closed".into()),
        Err(_) => Err("handshake timeout".into()),
    }
}
async fn send_json(socket: &mut WebSocket, message: &GatewayToAgent) -> anyhow::Result<()> {
    socket
        .send(Message::Text(serde_json::to_string(message)?.into()))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use futures_util::{SinkExt, StreamExt};
    use std::collections::BTreeMap;
    use tempfile::TempDir;
    use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};
    const TEST_TOKEN: &str = "developmenttoken123456";
    async fn spawn_test_gateway() -> (TempDir, AppState, SocketAddr, tokio::task::JoinHandle<()>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("infra.db")).unwrap();
        let state = AppState::new(store, Some(TEST_TOKEN.into()), None);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = build_app(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (dir, state, addr, task)
    }
    fn test_hello(device_id: &str, instance: Uuid) -> DeviceHello {
        DeviceHello {
            protocol: PROTOCOL_VERSION,
            device_id: device_id.into(),
            device_name: device_id.into(),
            agent_version: "test".into(),
            platform: "linux".into(),
            arch: "aarch64".into(),
            boot_id: "boot-test".into(),
            agent_instance_id: instance,
            capabilities: vec!["exec".into()],
            metadata: BTreeMap::new(),
        }
    }
    async fn connect_legacy(
        addr: SocketAddr,
        device_id: &str,
    ) -> (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        u64,
    ) {
        let (mut socket, _) = connect_async(format!("ws://{addr}/agent")).await.unwrap();
        let hello = AgentToGateway::LegacyHello(LegacyDeviceHello {
            hello: test_hello(device_id, Uuid::new_v4()),
            token: TEST_TOKEN.into(),
        });
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&hello).unwrap().into(),
            ))
            .await
            .unwrap();
        let ClientMessage::Text(text) = socket.next().await.unwrap().unwrap() else {
            panic!()
        };
        let GatewayToAgent::HelloAck(ack) = serde_json::from_str(text.as_str()).unwrap() else {
            panic!()
        };
        (socket, ack.connection_generation)
    }
    async fn wait_registered(state: &AppState, device_id: &str) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if state.devices.read().await.contains_key(device_id) {
                    break;
                }
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn legacy_handshake_and_exec_round_trip() {
        let (_dir, state, addr, server) = spawn_test_gateway().await;
        let (socket, generation) = connect_legacy(addr, "test-device").await;
        wait_registered(&state, "test-device").await;
        assert_eq!(generation, 1);
        let (mut sink, mut stream) = socket.split();
        let responder = tokio::spawn(async move {
            while let Some(Ok(ClientMessage::Text(text))) = stream.next().await {
                let message: GatewayToAgent = serde_json::from_str(text.as_str()).unwrap();
                if let GatewayToAgent::Request(request) = message {
                    let response = AgentToGateway::Response(AgentResponse {
                        id: request.id,
                        result: OperationResult::Ok {
                            body: serde_json::to_value(ExecOutput {
                                exit_code: Some(0),
                                stdout: "integration-ok\n".into(),
                                stderr: String::new(),
                                truncated: false,
                            })
                            .unwrap(),
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
        let response = dispatch_to_device(
            &state,
            "test-device".into(),
            AgentOperation::Exec {
                command: "ignored".into(),
                cwd: None,
                timeout_ms: Some(2000),
            },
            3000,
        )
        .await
        .unwrap();
        assert!(matches!(response.result, OperationResult::Ok { .. }));
        responder.abort();
        server.abort();
    }
    #[tokio::test]
    async fn signed_auth_and_revocation() {
        let (_dir, state, addr, server) = spawn_test_gateway().await;
        let enrollment = state.store.create_enrollment(60_000).unwrap();
        let secret = [9_u8; 32];
        let key = SigningKey::from_bytes(&secret);
        let enrolled = state
            .store
            .consume_enrollment(
                &enrollment.code,
                &Uuid::new_v4().to_string(),
                "signed",
                &key.verifying_key().to_bytes(),
                "linux",
                "aarch64",
                "test",
            )
            .unwrap();
        let (mut socket, _) = connect_async(format!("ws://{addr}/agent")).await.unwrap();
        let instance = Uuid::new_v4();
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&AgentToGateway::AuthStart(DeviceAuthStart {
                    protocol: PROTOCOL_VERSION,
                    device_id: enrolled.device_id.clone(),
                    agent_instance_id: instance,
                }))
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let ClientMessage::Text(text) = socket.next().await.unwrap().unwrap() else {
            panic!()
        };
        let GatewayToAgent::AuthChallenge(challenge) = serde_json::from_str(text.as_str()).unwrap()
        else {
            panic!()
        };
        let hello = test_hello(&enrolled.device_id, instance);
        let signature = URL_SAFE_NO_PAD.encode(
            key.sign(&signed_auth_message(
                &challenge.nonce,
                challenge.gateway_epoch,
                &hello,
            ))
            .to_bytes(),
        );
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(&AgentToGateway::SignedHello(SignedDeviceHello {
                    hello,
                    signature,
                }))
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let ClientMessage::Text(text) = socket.next().await.unwrap().unwrap() else {
            panic!()
        };
        assert!(matches!(
            serde_json::from_str::<GatewayToAgent>(text.as_str()).unwrap(),
            GatewayToAgent::HelloAck(_)
        ));
        state.store.revoke(&enrolled.device_id).unwrap();
        drop(socket);
        let (mut retry, _) = connect_async(format!("ws://{addr}/agent")).await.unwrap();
        retry
            .send(ClientMessage::Text(
                serde_json::to_string(&AgentToGateway::AuthStart(DeviceAuthStart {
                    protocol: PROTOCOL_VERSION,
                    device_id: enrolled.device_id,
                    agent_instance_id: Uuid::new_v4(),
                }))
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(1), retry.next())
            .await
            .unwrap();
        assert!(matches!(frame, Some(Ok(ClientMessage::Close(_))) | None));
        server.abort();
    }
    #[tokio::test]
    async fn newer_connection_supersedes_generation() {
        let (_dir, state, addr, server) = spawn_test_gateway().await;
        let (first, first_generation) = connect_legacy(addr, "same-device").await;
        wait_registered(&state, "same-device").await;
        let (second, second_generation) = connect_legacy(addr, "same-device").await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if state
                    .devices
                    .read()
                    .await
                    .get("same-device")
                    .map(|d| d.generation)
                    == Some(second_generation)
                {
                    break;
                }
                tokio::task::yield_now().await
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
