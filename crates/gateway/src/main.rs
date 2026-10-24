mod auth;
mod copy;
mod oauth;
mod pairing;
mod store;

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{
        Form, Path, Query, Request, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, VerifyingKey};
use hmac::{Hmac, Mac};
use infra_protocol::{
    AgentOperation, AgentRequest, AgentResponse, AgentToGateway, AuthChallenge, ContainerAction,
    ContainerRequest, DEFAULT_CONTROL_FRAME_LIMIT, DesktopAction, DesktopRequest, DeviceAuthStart,
    DeviceHello, ExecOutput, FileAction, FileRequest, GatewayToAgent, GitAction, GitRequest,
    HelloAck, JobAction, JobRequest, LegacyDeviceHello, NetworkAction, NetworkRequest,
    OperationResult, PROTOCOL_VERSION, ProcessAction, ProcessRequest, ReconcileSnapshot,
    SearchMode, SearchRequest, ServiceAction, ServiceRequest, SignedDeviceHello, SystemAction,
    SystemRequest, TerminalAction, TerminalRequest, TransferAction, TransferRequest,
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
use sha2::Sha256;
use std::{
    collections::HashMap,
    convert::Infallible,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use store::{DeviceRecord, Store, StoreError, now_ms};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot};
use tracing::{info, warn};
use uuid::Uuid;

const AGENT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const AGENT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);
const BROWSER_SESSION_COOKIE: &str = "infra_session";
const BROWSER_CSRF_HEADER: &str = "x-infra-csrf";
const BROWSER_SESSION_TTL_MS: u64 = 12 * 60 * 60 * 1000;

#[derive(Clone)]
struct AppState {
    legacy_agent_token: Option<Arc<str>>,
    admin_token: Option<Arc<str>>,
    owner_auth: Option<Arc<auth::OwnerAuth>>,
    oauth: oauth::OAuthState,
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
            owner_auth: None,
            oauth: oauth::OAuthState::default(),
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
    retire: mpsc::Sender<()>,
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
    connection_state: &'static str,
    last_disconnect_reason: Option<String>,
    last_disconnect_at_unix_ms: Option<u64>,
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
#[derive(Debug, Deserialize)]
struct PairRequestInput {
    request_id: String,
    device_name: String,
    public_key: String,
    platform: String,
    arch: String,
    agent_version: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
struct PairStatusInput {
    request_id: String,
    public_key: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
struct ApprovePairInput {
    verification_code: String,
}

#[derive(Debug, Deserialize)]
struct RenameDeviceInput {
    name: Option<String>,
}

#[derive(Deserialize)]
struct BrowserLoginInput {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Deserialize)]
struct ChangePasswordInput {
    current_password: String,
    new_password: String,
}

#[derive(Debug, Serialize)]
struct BrowserSessionView {
    authenticated: bool,
    mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthAuthorizeQuery {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    resource: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthAuthorizeForm {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    scope: String,
    resource: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenForm {
    grant_type: String,
    client_id: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    redirect_uri: Option<String>,
    #[serde(default)]
    code_verifier: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    resource: Option<String>,
}

#[derive(Debug)]
enum DispatchError {
    NotFound(String),
    Ambiguous {
        selector: String,
        matches: Vec<String>,
    },
    Revoked(String),
    Offline(String),
    Storage(String),
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
struct McpCopyInput {
    source_device: String,
    destination_device: String,
    source_path: String,
    destination_path: String,
    #[serde(default)]
    overwrite: Option<bool>,
    #[serde(default)]
    create_parents: Option<bool>,
    #[serde(default)]
    source_transfer_id: Option<String>,
    #[serde(default)]
    destination_transfer_id: Option<String>,
    /// Max bytes relayed this request (default 16 MiB, cap 64 MiB).
    #[serde(default)]
    max_bytes: Option<u64>,
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

#[derive(Debug, Deserialize, JsonSchema)]
struct McpDesktopInput {
    device: String,
    action: String,
    #[serde(default)]
    display: Option<u32>,
    #[serde(default)]
    x: Option<i32>,
    #[serde(default)]
    y: Option<i32>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    app: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    modifiers: Option<Vec<String>>,
    #[serde(default)]
    button: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpSystemInput {
    device: String,
    action: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpNetworkInput {
    device: String,
    action: String,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpServiceInput {
    device: String,
    action: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    query: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpContainerInput {
    device: String,
    action: String,
    #[serde(default)]
    container: Option<String>,
    #[serde(default)]
    tail: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpGitInput {
    device: String,
    action: String,
    repo: String,
    #[serde(default)]
    staged: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    remote: Option<String>,
    #[serde(default)]
    branch: Option<String>,
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
        let device_id = resolve_device(&self.state, &input.device)
            .await
            .map_err(dispatch_error)?;
        let response = dispatch_to_device(
            &self.state,
            device_id.clone(),
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

    #[tool(
        name = "copy",
        description = "Copy a file between enrolled Infra devices by name or ID. SHA-256 verified, resumable, bounded to 16 MiB per call by default. For partial results call again with both transfer IDs. Never auto-retry an uncertain chunk."
    )]
    async fn copy(
        &self,
        Parameters(input): Parameters<McpCopyInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        Ok(McpJson(copy::run(&self.state, input).await?))
    }

    #[tool(
        name = "desktop",
        description = "Desktop control on an Infra device. Actions: permissions, displays, windows, screenshot, focus, click, type_text, key, clipboard_get, clipboard_set."
    )]
    async fn desktop(
        &self,
        Parameters(input): Parameters<McpDesktopInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let action = match input.action.as_str() {
            "permissions" => DesktopAction::Permissions,
            "displays" => DesktopAction::Displays,
            "windows" => DesktopAction::Windows,
            "screenshot" => DesktopAction::Screenshot {
                display: input.display,
                x: input.x,
                y: input.y,
                width: input.width,
                height: input.height,
            },
            "focus" => DesktopAction::Focus {
                app: required(input.app, "app")?,
            },
            "click" => DesktopAction::Click {
                x: input.x.ok_or("x is required")?,
                y: input.y.ok_or("y is required")?,
                button: input.button,
            },
            "type_text" => DesktopAction::TypeText {
                text: required(input.text, "text")?,
            },
            "key" => DesktopAction::Key {
                key: required(input.key, "key")?,
                modifiers: input.modifiers.unwrap_or_default(),
            },
            "clipboard_get" => DesktopAction::ClipboardGet,
            "clipboard_set" => DesktopAction::ClipboardSet {
                text: required(input.text, "text")?,
            },
            _ => return Err("unknown desktop action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Desktop(DesktopRequest { action }),
            65_000,
        )
        .await
    }

    #[tool(
        name = "system",
        description = "Structured system information. Actions: summary, disks, networks."
    )]
    async fn system(
        &self,
        Parameters(input): Parameters<McpSystemInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let action = match input.action.as_str() {
            "summary" => SystemAction::Summary,
            "disks" => SystemAction::Disks,
            "networks" => SystemAction::Networks,
            _ => return Err("unknown system action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::System(SystemRequest { action }),
            30_000,
        )
        .await
    }

    #[tool(
        name = "network",
        description = "Structured network diagnostics. Actions: listeners, resolve, tcp_check."
    )]
    async fn network(
        &self,
        Parameters(input): Parameters<McpNetworkInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let action = match input.action.as_str() {
            "listeners" => NetworkAction::Listeners,
            "resolve" => NetworkAction::Resolve {
                host: required(input.host, "host")?,
            },
            "tcp_check" => NetworkAction::TcpCheck {
                host: required(input.host, "host")?,
                port: input.port.ok_or("port is required")?,
                timeout_ms: input.timeout_ms,
            },
            _ => return Err("unknown network action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Network(NetworkRequest { action }),
            45_000,
        )
        .await
    }

    #[tool(
        name = "services",
        description = "User-level/native service control without shell interpolation. Actions: list, status, start, stop, restart."
    )]
    async fn services(
        &self,
        Parameters(input): Parameters<McpServiceInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let action = match input.action.as_str() {
            "list" => ServiceAction::List { query: input.query },
            "status" => ServiceAction::Status {
                name: required(input.name, "name")?,
            },
            "start" => ServiceAction::Start {
                name: required(input.name, "name")?,
            },
            "stop" => ServiceAction::Stop {
                name: required(input.name, "name")?,
            },
            "restart" => ServiceAction::Restart {
                name: required(input.name, "name")?,
            },
            _ => return Err("unknown services action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Services(ServiceRequest { action }),
            65_000,
        )
        .await
    }

    #[tool(
        name = "containers",
        description = "Docker container operations using direct argv execution. Actions: list, inspect, logs, start, stop, restart."
    )]
    async fn containers(
        &self,
        Parameters(input): Parameters<McpContainerInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let action = match input.action.as_str() {
            "list" => ContainerAction::List,
            "inspect" => ContainerAction::Inspect {
                container: required(input.container, "container")?,
            },
            "logs" => ContainerAction::Logs {
                container: required(input.container, "container")?,
                tail: input.tail,
            },
            "start" => ContainerAction::Start {
                container: required(input.container, "container")?,
            },
            "stop" => ContainerAction::Stop {
                container: required(input.container, "container")?,
            },
            "restart" => ContainerAction::Restart {
                container: required(input.container, "container")?,
            },
            _ => return Err("unknown containers action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Containers(ContainerRequest { action }),
            90_000,
        )
        .await
    }

    #[tool(
        name = "git",
        description = "Structured Git operations without a shell. Actions: status, diff, log, branches, remotes, fetch, pull, push."
    )]
    async fn git(
        &self,
        Parameters(input): Parameters<McpGitInput>,
    ) -> Result<McpJson<serde_json::Value>, String> {
        let action = match input.action.as_str() {
            "status" => GitAction::Status { repo: input.repo },
            "diff" => GitAction::Diff {
                repo: input.repo,
                staged: input.staged.unwrap_or(false),
            },
            "log" => GitAction::Log {
                repo: input.repo,
                limit: input.limit,
            },
            "branches" => GitAction::Branches { repo: input.repo },
            "remotes" => GitAction::Remotes { repo: input.repo },
            "fetch" => GitAction::Fetch {
                repo: input.repo,
                remote: input.remote,
            },
            "pull" => GitAction::Pull {
                repo: input.repo,
                remote: input.remote,
                branch: input.branch,
            },
            "push" => GitAction::Push {
                repo: input.repo,
                remote: input.remote,
                branch: input.branch,
            },
            _ => return Err("unknown git action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Git(GitRequest { action }),
            130_000,
        )
        .await
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if auth::provision_from_cli()? {
        return Ok(());
    }
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
    let admin = std::env::var("INFRA_ADMIN_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    if admin.as_deref().is_some_and(|value| value.len() < 32) {
        anyhow::bail!("INFRA_ADMIN_TOKEN must be at least 32 bytes when configured");
    }
    if !bind.ip().is_loopback() && admin.is_none() {
        anyhow::bail!(
            "refusing non-loopback bind without INFRA_ADMIN_TOKEN; terminate TLS before the gateway and configure an admin credential"
        );
    }
    let db_path = PathBuf::from(
        std::env::var("INFRA_DB_PATH").unwrap_or_else(|_| "./data/infra-v1.db".into()),
    );
    let store = Store::open(db_path)?;
    let legacy = std::env::var("INFRA_LEGACY_AGENT_TOKEN")
        .ok()
        .or_else(|| std::env::var("INFRA_AGENT_TOKEN").ok());
    let mut state = AppState::new(store, legacy, admin);
    if let Ok(path) = std::env::var("INFRA_DASHBOARD_AUTH_FILE") {
        state.owner_auth = Some(Arc::new(auth::OwnerAuth::load(PathBuf::from(path))?));
    }
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
                .with_allowed_hosts([
                    "localhost",
                    "127.0.0.1",
                    "::1",
                    "100.79.230.22",
                    "100.79.230.22:18790",
                    "infra.drbijoy.in",
                ])
                .with_legacy_session_mode(false)
                .with_json_response(true),
        );
    let mcp = Router::new().nest_service("/mcp", mcp_service).route_layer(
        middleware::from_fn_with_state(state.clone(), require_mcp_auth),
    );
    let protected_api = Router::new()
        .route("/api/overview", get(overview_http))
        .route("/api/devices", get(list_devices_http))
        .route("/api/sessions", get(list_sessions_http))
        .route("/api/activity", get(list_activity_http))
        .route("/api/events", get(dashboard_events))
        .route("/api/enrollments", post(create_enrollment))
        .route("/api/pair/requests", get(list_pending_pairs))
        .route("/api/pair/{request_id}/approve", post(approve_pair))
        .route("/api/pair/{request_id}/reject", post(reject_pair))
        .route("/api/devices/{device_id}/rename", post(rename_device))
        .route("/api/auth/password", post(change_browser_password))
        .route("/api/devices/{device_id}/revoke", post(revoke_device))
        .route(
            "/api/devices/{device_id}/operations",
            post(dashboard_operation),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_auth,
        ));
    Router::new()
        .merge(mcp)
        .merge(protected_api)
        .route("/", get(dashboard_index))
        .route("/app.js", get(dashboard_js))
        .route("/styles.css", get(dashboard_css))
        .route("/vendor/xterm.js", get(dashboard_terminal_js))
        .route("/vendor/xterm.css", get(dashboard_terminal_css))
        .route("/assets/logo.png", get(dashboard_logo))
        .route("/favicon.png", get(dashboard_favicon))
        .route("/favicon.ico", get(dashboard_favicon_ico))
        .route("/apple-touch-icon.png", get(dashboard_touch_icon))
        .route("/downloads/install.sh", get(download_install_sh))
        .route("/downloads/install.ps1", get(download_install_ps1))
        .route("/install.sh", get(download_install_sh))
        .route("/install.ps1", get(download_install_ps1))
        .route("/healthz", get(health))
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth_protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_authorization_server_metadata),
        )
        .route("/oauth/register", post(oauth_register))
        .route(
            "/oauth/authorize",
            get(oauth_authorize_get).post(oauth_authorize_post),
        )
        .route("/oauth/token", post(oauth_token))
        .route(
            "/api/auth/session",
            get(browser_session_status)
                .post(browser_login)
                .delete(browser_logout),
        )
        .route("/enroll", post(enroll))
        .route("/pair/request", post(request_pair))
        .route("/pair/status", post(pair_status))
        .route("/agent", get(agent_upgrade))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            dashboard_edge,
        ))
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
async fn dashboard_logo() -> Response {
    image_response(
        include_bytes!("../../../apps/dashboard/assets/logo.png"),
        "image/png",
    )
}
async fn dashboard_favicon() -> Response {
    image_response(
        include_bytes!("../../../apps/dashboard/assets/favicon.png"),
        "image/png",
    )
}
async fn dashboard_favicon_ico() -> Response {
    image_response(
        include_bytes!("../../../apps/dashboard/assets/favicon.ico"),
        "image/x-icon",
    )
}
async fn dashboard_touch_icon() -> Response {
    image_response(
        include_bytes!("../../../apps/dashboard/assets/apple-touch-icon.png"),
        "image/png",
    )
}
fn image_response(data: &'static [u8], content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        data,
    )
        .into_response()
}

fn oauth_origin(state: &AppState) -> Option<String> {
    state.owner_auth.as_ref().map(|owner| owner.public_origin())
}

fn oauth_resource(state: &AppState) -> Option<String> {
    oauth_origin(state).map(|origin| format!("{origin}/mcp"))
}

fn oauth_signing_secret(state: &AppState) -> Option<String> {
    state
        .admin_token
        .as_deref()
        .map(str::to_owned)
        .or_else(|| browser_session_secret(state))
}

fn oauth_json_error(status: StatusCode, code: &str, description: &str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": code,
            "error_description": description,
        })),
    )
        .into_response()
}

async fn oauth_protected_resource_metadata(State(state): State<AppState>) -> Response {
    let Some(origin) = oauth_origin(&state) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth is not configured").into_response();
    };
    let resource = format!("{origin}/mcp");
    Json(serde_json::json!({
        "resource": resource,
        "authorization_servers": [origin.clone()],
        "scopes_supported": [oauth::CONTROL_SCOPE, oauth::OFFLINE_SCOPE],
        "resource_documentation": origin,
    }))
    .into_response()
}

async fn oauth_authorization_server_metadata(State(state): State<AppState>) -> Response {
    let Some(origin) = oauth_origin(&state) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth is not configured").into_response();
    };
    Json(serde_json::json!({
        "issuer": origin,
        "authorization_endpoint": format!("{origin}/oauth/authorize"),
        "token_endpoint": format!("{origin}/oauth/token"),
        "registration_endpoint": format!("{origin}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": [oauth::CONTROL_SCOPE, oauth::OFFLINE_SCOPE],
        "authorization_response_iss_parameter_supported": true,
    }))
    .into_response()
}

async fn oauth_register(
    State(state): State<AppState>,
    Json(input): Json<oauth::RegistrationRequest>,
) -> Response {
    let Some(secret) = oauth_signing_secret(&state) else {
        return oauth_json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "OAuth is not configured",
        );
    };
    match oauth::register_client(secret.as_bytes(), input) {
        Ok(registration) => (StatusCode::CREATED, Json(registration)).into_response(),
        Err(error) => oauth_json_error(StatusCode::BAD_REQUEST, "invalid_client_metadata", error),
    }
}

fn validate_oauth_authorize_request(
    state: &AppState,
    query: &OAuthAuthorizeQuery,
) -> Result<(String, String), &'static str> {
    if query.response_type != "code" {
        return Err("response_type must be code");
    }
    if query.code_challenge_method != "S256" || query.code_challenge.len() != 43 {
        return Err("PKCE S256 is required");
    }
    let secret = oauth_signing_secret(state).ok_or("OAuth is not configured")?;
    let registration = oauth::validate_client(secret.as_bytes(), &query.client_id)
        .ok_or("unknown or expired client_id")?;
    if !registration
        .redirect_uris
        .iter()
        .any(|uri| uri == &query.redirect_uri)
    {
        return Err("redirect_uri is not registered for this client");
    }
    let resource = query
        .resource
        .clone()
        .unwrap_or_else(|| oauth_resource(state).unwrap_or_default());
    if Some(resource.as_str()) != oauth_resource(state).as_deref() {
        return Err("resource does not match this MCP server");
    }
    let scope = oauth::normalize_scope(query.scope.as_deref())?;
    Ok((scope, resource))
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn oauth_authorize_page(
    query: &OAuthAuthorizeQuery,
    scope: &str,
    resource: &str,
    origin: &str,
    signed_in: bool,
) -> Response {
    let hidden = |name: &str, value: &str| {
        format!(
            "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
            html_escape(name),
            html_escape(value)
        )
    };
    let mut fields = String::new();
    fields.push_str(&hidden("response_type", &query.response_type));
    fields.push_str(&hidden("client_id", &query.client_id));
    fields.push_str(&hidden("redirect_uri", &query.redirect_uri));
    fields.push_str(&hidden("code_challenge", &query.code_challenge));
    fields.push_str(&hidden(
        "code_challenge_method",
        &query.code_challenge_method,
    ));
    fields.push_str(&hidden("scope", scope));
    fields.push_str(&hidden("resource", resource));
    if let Some(state) = &query.state {
        fields.push_str(&hidden("state", state));
    }
    let credentials = if signed_in {
        "<p class=\"signed\">Signed in to Infra. Continue to authorize ChatGPT.</p>".to_string()
    } else {
        "<label>Username<input name=\"username\" autocomplete=\"username\" required></label><label>Password<input name=\"password\" type=\"password\" autocomplete=\"current-password\" required></label>".to_string()
    };
    let body = format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Authorize ChatGPT · Infra</title>
<style>body{{margin:0;background:#090a0c;color:#f5f6f7;font:15px/1.45 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;display:grid;place-items:center;min-height:100vh}}main{{width:min(430px,calc(100vw - 40px));background:#111318;border:1px solid #272a31;border-radius:20px;padding:28px;box-shadow:0 24px 80px #0008}}.mark{{width:44px;height:44px;border-radius:12px;background:#f4f4f4 url('/assets/logo.png') center/34px 34px no-repeat;margin-bottom:18px}}h1{{font-size:24px;margin:0 0 8px}}p{{color:#a9adb7;margin:0 0 20px}}.scope{{padding:13px 14px;background:#0c0e12;border:1px solid #24272d;border-radius:12px;margin:18px 0;color:#dfe2e7}}label{{display:block;color:#b8bcc5;margin:14px 0 6px}}input{{display:block;box-sizing:border-box;width:100%;margin-top:7px;background:#0b0d10;color:white;border:1px solid #30343b;border-radius:10px;padding:12px 13px;font:inherit;outline:none}}input:focus{{border-color:#747b89}}button{{width:100%;margin-top:18px;padding:13px 16px;border:0;border-radius:11px;background:white;color:#090a0c;font-weight:700;font:inherit;cursor:pointer}}small{{display:block;margin-top:16px;color:#777d89}}.signed{{padding:12px 13px;border-radius:10px;background:#101a14;color:#bfe5c9;border:1px solid #24452d}}</style></head>
<body><main><div class="mark"></div><h1>Authorize ChatGPT</h1><p>ChatGPT wants permission to control your Infra-connected devices through MCP.</p><div class="scope"><strong>Requested access</strong><br>{scope}</div><form method="post" action="{origin}/oauth/authorize">{fields}{credentials}<button type="submit">Authorize ChatGPT</button></form><small>Only authorize this connection if you initiated it from ChatGPT.</small></main></body></html>"#,
        scope = html_escape(scope),
        fields = fields,
        credentials = credentials,
        origin = html_escape(origin)
    );
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    let csp = String::from(
        "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src https://static.cloudflareinsights.com; connect-src https://cloudflareinsights.com; form-action https: http://127.0.0.1:* http://localhost:* http://[::1]:*; base-uri 'none'; frame-ancestors 'none'",
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(&csp) {
        response
            .headers_mut()
            .insert(header::CONTENT_SECURITY_POLICY, value);
    }
    response
}

async fn oauth_authorize_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<OAuthAuthorizeQuery>,
) -> Response {
    let (scope, resource) = match validate_oauth_authorize_request(&state, &query) {
        Ok(value) => value,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let signed_in = browser_session_expiry(&state, &headers).is_some();
    let Some(origin) = oauth_origin(&state) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth is not configured").into_response();
    };
    oauth_authorize_page(&query, &scope, &resource, &origin, signed_in)
}

async fn oauth_authorize_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(input): Form<OAuthAuthorizeForm>,
) -> Response {
    let query = OAuthAuthorizeQuery {
        response_type: input.response_type,
        client_id: input.client_id,
        redirect_uri: input.redirect_uri,
        code_challenge: input.code_challenge,
        code_challenge_method: input.code_challenge_method,
        scope: Some(input.scope),
        state: input.state,
        resource: Some(input.resource),
    };
    let (scope, resource) = match validate_oauth_authorize_request(&state, &query) {
        Ok(value) => value,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };

    if browser_session_expiry(&state, &headers).is_none() {
        let Some(owner) = &state.owner_auth else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Owner login is not configured",
            )
                .into_response();
        };
        if !owner.admit_attempt() {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, "60")],
                "Too many sign-in attempts. Try again shortly.",
            )
                .into_response();
        }
        let Ok(permit) = owner.verification_slots.clone().try_acquire_owned() else {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Sign-in is busy. Try again shortly.",
            )
                .into_response();
        };
        let owner = owner.clone();
        let username = input.username.unwrap_or_default();
        let password = input.password.unwrap_or_default();
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            owner.verify(&username, &password)
        })
        .await
        .unwrap_or(false);
        if !valid {
            return (StatusCode::UNAUTHORIZED, "Incorrect username or password.").into_response();
        }
    }

    let code = match state
        .oauth
        .issue_code(oauth::AuthorizationCode {
            client_id: query.client_id.clone(),
            redirect_uri: query.redirect_uri.clone(),
            code_challenge: query.code_challenge.clone(),
            scope,
            resource,
            expires_at: oauth::code_expiry(),
        })
        .await
    {
        Ok(code) => code,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let Some(issuer) = oauth_origin(&state) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Ok(mut redirect) = url::Url::parse(&query.redirect_uri) else {
        return (StatusCode::BAD_REQUEST, "invalid redirect_uri").into_response();
    };
    {
        let mut pairs = redirect.query_pairs_mut();
        pairs.append_pair("code", &code);
        if let Some(state_value) = &query.state {
            pairs.append_pair("state", state_value);
        }
        pairs.append_pair("iss", &issuer);
    }
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, redirect.to_string())],
    )
        .into_response()
}

async fn oauth_token(State(state): State<AppState>, Form(input): Form<OAuthTokenForm>) -> Response {
    let Some(secret) = oauth_signing_secret(&state) else {
        return oauth_json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "OAuth is not configured",
        );
    };
    if oauth::validate_client(secret.as_bytes(), &input.client_id).is_none() {
        return oauth_json_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "Unknown or expired client_id",
        );
    }
    let Some(expected_resource) = oauth_resource(&state) else {
        return oauth_json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "OAuth is not configured",
        );
    };
    if let Some(resource) = &input.resource
        && resource != &expected_resource
    {
        return oauth_json_error(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            "Resource does not match this MCP server",
        );
    }

    match input.grant_type.as_str() {
        "authorization_code" => {
            let Some(code_value) = input.code.as_deref() else {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "code is required",
                );
            };
            let Some(code) = state.oauth.take_code(code_value).await else {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "Authorization code is invalid or expired",
                );
            };
            if code.client_id != input.client_id
                || input.redirect_uri.as_deref() != Some(code.redirect_uri.as_str())
                || code.resource != expected_resource
            {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "Authorization code binding mismatch",
                );
            }
            let Some(verifier) = input.code_verifier.as_deref() else {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "code_verifier is required",
                );
            };
            if !oauth::verify_pkce(verifier, &code.code_challenge) {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "PKCE verification failed",
                );
            }
            match oauth::issue_tokens(
                secret.as_bytes(),
                &input.client_id,
                &code.scope,
                &expected_resource,
            ) {
                Ok(tokens) => Json(tokens).into_response(),
                Err(error) => {
                    oauth_json_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", error)
                }
            }
        }
        "refresh_token" => {
            let Some(refresh_token) = input.refresh_token.as_deref() else {
                return oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "refresh_token is required",
                );
            };
            match oauth::refresh_tokens(
                secret.as_bytes(),
                refresh_token,
                &input.client_id,
                &expected_resource,
            ) {
                Ok(tokens) => Json(tokens).into_response(),
                Err(error) => oauth_json_error(StatusCode::BAD_REQUEST, "invalid_grant", error),
            }
        }
        _ => oauth_json_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "Only authorization_code and refresh_token are supported",
        ),
    }
}

async fn dashboard_edge(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if let Some(owner) = &state.owner_auth {
        let origin = owner.public_origin();
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let public_host = origin.strip_prefix("https://").unwrap_or("");
        if host.eq_ignore_ascii_case(public_host) && !secure_cookie_requested(request.headers()) {
            if request.method() == axum::http::Method::GET
                || request.method() == axum::http::Method::HEAD
            {
                let target = format!("{}{}", origin, request.uri());
                return (StatusCode::PERMANENT_REDIRECT, [(header::LOCATION, target)])
                    .into_response();
            }
            return (StatusCode::FORBIDDEN, "HTTPS required").into_response();
        }
        if let Some(request_origin) = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
        {
            let direct = format!("http://{host}");
            if request_origin != origin
                && (host.eq_ignore_ascii_case(public_host) || request_origin != direct)
            {
                return (StatusCode::FORBIDDEN, "cross-origin request rejected").into_response();
            }
        }
    }
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
async fn dashboard_terminal_js() -> Response {
    static_response(
        include_str!("../../../apps/dashboard/vendor/xterm.js"),
        "text/javascript; charset=utf-8",
    )
}
async fn dashboard_terminal_css() -> Response {
    static_response(
        include_str!("../../../apps/dashboard/vendor/xterm.css"),
        "text/css; charset=utf-8",
    )
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
                // xterm renders its grid using generated styles. Script execution
                // remains self-only; no remote assets or inline scripts are allowed.
                "default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
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

// The browser and MCP use the same typed operations, dispatcher, durable workers
// and audit path. Keep the browser surface limited to the implemented controls.
async fn dashboard_operation(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(operation): Json<AgentOperation>,
) -> Response {
    let supported = matches!(
        &operation,
        AgentOperation::Status
            | AgentOperation::Desktop(DesktopRequest {
                action: DesktopAction::Permissions
            })
            | AgentOperation::Terminal(_)
            | AgentOperation::Jobs(JobRequest {
                action: JobAction::List | JobAction::Read { .. } | JobAction::Cancel { .. }
            })
            | AgentOperation::Files(FileRequest {
                action: FileAction::List { .. }
                    | FileAction::Read { .. }
                    | FileAction::Stat { .. }
                    | FileAction::Hash { .. }
            })
            | AgentOperation::Transfer(TransferRequest {
                action: TransferAction::List
                    | TransferAction::Status { .. }
                    | TransferAction::Cancel { .. }
            })
    );
    let response = if !supported {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"operation is not available in the dashboard"})),
        )
            .into_response()
    } else {
        match tool_operation(&state, device_id, operation, 15_000).await {
            Ok(McpJson(value)) => Json(value).into_response(),
            Err(error) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error":error})),
            )
                .into_response(),
        }
    };
    let mut response = response;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
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
                device_name: record
                    .display_name
                    .clone()
                    .unwrap_or_else(|| record.device_name.clone()),
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
// Single bounded snapshot avoids three independent HTTP round trips on each UI refresh.
async fn overview_http(State(state): State<AppState>) -> Response {
    let devices = device_views(&state).await;
    let sessions = dashboard_sessions(&state).await;
    let activity = state.store.list_activity(100);
    match (devices, sessions, activity) {
        (Ok(devices), Ok(sessions), Ok(activity)) => Json(serde_json::json!({
            "devices": devices,
            "sessions": sessions,
            "activity": activity,
        }))
        .into_response(),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error":"overview snapshot unavailable"})),
        )
            .into_response(),
    }
}
async fn list_devices_http(State(state): State<AppState>) -> Response {
    match device_views(&state).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn create_enrollment(
    State(state): State<AppState>,
    Json(input): Json<CreateEnrollmentInput>,
) -> Response {
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

// Proof of possession for an untrusted candidate key. Approval grants trust.
fn verify_pair_signature(key: &str, signature: &str, message: &[u8]) -> Option<[u8; 32]> {
    let bytes = URL_SAFE_NO_PAD.decode(key).ok()?;
    let key_bytes: [u8; 32] = bytes.try_into().ok()?;
    let verifying = VerifyingKey::from_bytes(&key_bytes).ok()?;
    let sig: [u8; 64] = URL_SAFE_NO_PAD.decode(signature).ok()?.try_into().ok()?;
    verifying
        .verify_strict(message, &Signature::from_bytes(&sig))
        .ok()?;
    Some(key_bytes)
}

fn pair_field_valid(field: &str, maximum: usize) -> bool {
    !field.trim().is_empty() && field.len() <= maximum && !field.contains(['\r', '\n', '\0'])
}

fn pair_request_message(input: &PairRequestInput) -> String {
    format!(
        "infra-pair-request-v1\n{}\n{}\n{}\n{}\n{}\n{}",
        input.request_id,
        input.public_key,
        input.device_name,
        input.platform,
        input.arch,
        input.agent_version,
    )
}

async fn request_pair(
    State(state): State<AppState>,
    Json(input): Json<PairRequestInput>,
) -> Response {
    if Uuid::parse_str(&input.request_id).is_err()
        || !pair_field_valid(&input.device_name, 120)
        || !pair_field_valid(&input.platform, 32)
        || !pair_field_valid(&input.arch, 32)
        || !pair_field_valid(&input.agent_version, 64)
    {
        return (StatusCode::BAD_REQUEST, "invalid pairing request").into_response();
    }
    let Some(key) = verify_pair_signature(
        &input.public_key,
        &input.signature,
        pair_request_message(&input).as_bytes(),
    ) else {
        return (StatusCode::UNAUTHORIZED, "invalid pairing key proof").into_response();
    };
    match state.store.request_pair(
        &input.request_id,
        &key,
        &input.device_name,
        &input.platform,
        &input.arch,
        &input.agent_version,
    ) {
        Ok(record) => {
            state.emit("pair_requested", None);
            Json(record).into_response()
        }
        Err(StoreError::PairCapacity) => (
            StatusCode::TOO_MANY_REQUESTS,
            "pairing is busy; try again later",
        )
            .into_response(),
        Err(StoreError::PairConflict) => {
            (StatusCode::CONFLICT, "pairing identity conflict").into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn pair_status(
    State(state): State<AppState>,
    Json(input): Json<PairStatusInput>,
) -> Response {
    if Uuid::parse_str(&input.request_id).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let message = format!("infra-pair-status-v1\n{}", input.request_id);
    let Some(key) = verify_pair_signature(&input.public_key, &input.signature, message.as_bytes())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match state.store.pair_status(&input.request_id, &key) {
        Ok(record) => Json(record).into_response(),
        Err(StoreError::PairInvalid) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn list_pending_pairs(State(state): State<AppState>) -> Response {
    match state.store.pending_pairs() {
        Ok(records) => Json(records).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn approve_pair(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    Json(input): Json<ApprovePairInput>,
) -> Response {
    if Uuid::parse_str(&request_id).is_err()
        || input.verification_code.len() != 6
        || !input.verification_code.bytes().all(|b| b.is_ascii_digit())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match state
        .store
        .approve_pair(&request_id, &input.verification_code)
    {
        Ok(record) => {
            let _ = state.store.record_activity(
                record.device_id.as_deref(),
                "pair.approve",
                "ok",
                Some(&request_id),
            );
            state.emit("pair_approved", record.device_id.clone());
            Json(record).into_response()
        }
        Err(StoreError::PairInvalid) => StatusCode::NOT_FOUND.into_response(),
        Err(StoreError::PairMismatch) => StatusCode::FORBIDDEN.into_response(),
        Err(StoreError::PairExpired) => StatusCode::GONE.into_response(),
        Err(StoreError::PairConflict) => StatusCode::CONFLICT.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn reject_pair(State(state): State<AppState>, Path(request_id): Path<String>) -> Response {
    if Uuid::parse_str(&request_id).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match state.store.reject_pair(&request_id) {
        Ok(true) => {
            state.emit("pair_rejected", None);
            Json(serde_json::json!({"ok":true})).into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn rename_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(input): Json<RenameDeviceInput>,
) -> Response {
    match state
        .store
        .set_device_alias(&device_id, input.name.as_deref())
    {
        Ok(true) => {
            state.emit("device_renamed", Some(device_id.clone()));
            Json(serde_json::json!({"ok":true,"device_id":device_id})).into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(StoreError::PairInvalid) => {
            (StatusCode::BAD_REQUEST, "invalid display name").into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn revoke_device(State(state): State<AppState>, Path(device_id): Path<String>) -> Response {
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

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (&a, &b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

fn bearer_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(expected) = &state.admin_token else {
        return state.owner_auth.is_none();
    };
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|provided| constant_time_eq(provided.as_bytes(), expected.as_bytes()))
}

fn browser_cookie(headers: &HeaderMap) -> Option<&str> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == BROWSER_SESSION_COOKIE).then_some(value)
    })
}

type HmacSha256 = Hmac<Sha256>;

fn signed_session_message(expires_at_unix_ms: u64, nonce: &str) -> String {
    format!("infra-browser-session-v1|{expires_at_unix_ms}|{nonce}")
}

fn browser_session_signature(secret: &[u8], expires_at_unix_ms: u64, nonce: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts arbitrary key length");
    mac.update(signed_session_message(expires_at_unix_ms, nonce).as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

fn create_browser_session_cookie(secret: &[u8]) -> Result<(String, u64), &'static str> {
    let mut nonce = [0_u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| "OS random source unavailable")?;
    let nonce = URL_SAFE_NO_PAD.encode(nonce);
    let expires = now_ms().saturating_add(BROWSER_SESSION_TTL_MS);
    let signature = browser_session_signature(secret, expires, &nonce);
    Ok((format!("v1.{expires}.{nonce}.{signature}"), expires))
}

fn browser_session_expiry(state: &AppState, headers: &HeaderMap) -> Option<u64> {
    let secret = browser_session_secret(state)?;
    let cookie = browser_cookie(headers)?;
    let mut fields = cookie.split('.');
    if fields.next()? != "v1" {
        return None;
    }
    let expires = fields.next()?.parse::<u64>().ok()?;
    let nonce = fields.next()?;
    let signature = fields.next()?;
    if fields.next().is_some() || nonce.len() > 128 || expires <= now_ms() {
        return None;
    }
    let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(signed_session_message(expires, nonce).as_bytes());
    mac.verify_slice(&signature).ok()?;
    Some(expires)
}

async fn require_mcp_auth(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let oauth_ok = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| {
            let Some(secret) = oauth_signing_secret(&state) else {
                return false;
            };
            let Some(resource) = oauth_resource(&state) else {
                return false;
            };
            oauth::validate_access_token(secret.as_bytes(), token, &resource)
        });
    if bearer_authorized(&state, headers) || oauth_ok {
        return next.run(request).await;
    }
    let metadata = oauth_origin(&state)
        .map(|origin| format!("{origin}/.well-known/oauth-protected-resource"))
        .unwrap_or_default();
    let challenge = format!(
        "Bearer resource_metadata=\"{metadata}\", scope=\"{}\"",
        oauth::CONTROL_SCOPE
    );
    let mut response = (StatusCode::UNAUTHORIZED, "OAuth authorization required").into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(&challenge) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

async fn require_api_auth(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if bearer_authorized(&state, request.headers()) {
        return next.run(request).await;
    }
    if browser_session_expiry(&state, request.headers()).is_none() {
        return (StatusCode::UNAUTHORIZED, "dashboard authorization required").into_response();
    }
    let safe_method = matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    let csrf_ok = request
        .headers()
        .get(BROWSER_CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some("1");
    if safe_method || csrf_ok {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "CSRF header required").into_response()
    }
}

fn secure_cookie_requested(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("https"))
}

fn session_cookie(value: &str, max_age_seconds: u64, secure: bool) -> String {
    format!(
        "{BROWSER_SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age_seconds}{}",
        if secure { "; Secure" } else { "" }
    )
}

async fn browser_session_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let open = state.admin_token.is_none() && state.owner_auth.is_none();
    let bearer = bearer_authorized(&state, &headers);
    let expires = browser_session_expiry(&state, &headers);
    Json(serde_json::json!({
        "authenticated":open || bearer || expires.is_some(),
        "mode":if open { "open_loopback" } else if expires.is_some() { "browser_session" } else if bearer { "bearer" } else { "none" },
        "expires_at_unix_ms":expires,
        "login_method":if state.owner_auth.is_some() { "password" } else { "token" }
    }))
}

fn browser_session_secret(state: &AppState) -> Option<String> {
    state
        .owner_auth
        .as_ref()
        .map(|owner| owner.session_secret())
        .or_else(|| state.admin_token.as_deref().map(str::to_owned))
}

async fn browser_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<BrowserLoginInput>,
) -> Response {
    if let Some(owner) = &state.owner_auth {
        if headers
            .get(BROWSER_CSRF_HEADER)
            .and_then(|v| v.to_str().ok())
            != Some("1")
        {
            return (StatusCode::FORBIDDEN, "CSRF header required").into_response();
        }
        if !owner.admit_attempt() {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, "60")],
                "Too many sign-in attempts. Try again shortly.",
            )
                .into_response();
        }
        let Ok(permit) = owner.verification_slots.clone().try_acquire_owned() else {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Sign-in is busy. Try again shortly.",
            )
                .into_response();
        };
        let owner = owner.clone();
        let username = input.username.unwrap_or_default();
        let password = input.password.unwrap_or_default();
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            owner.verify(&username, &password)
        })
        .await
        .unwrap_or(false);
        if !valid {
            return (StatusCode::UNAUTHORIZED, "Incorrect username or password.").into_response();
        }
    } else if let Some(expected) = &state.admin_token {
        if !constant_time_eq(
            input.token.unwrap_or_default().as_bytes(),
            expected.as_bytes(),
        ) {
            return (StatusCode::UNAUTHORIZED, "invalid admin credential").into_response();
        }
    } else {
        return Json(BrowserSessionView {
            authenticated: true,
            mode: "open_loopback",
            expires_at_unix_ms: None,
        })
        .into_response();
    }
    issue_browser_session(&state, &headers)
}

fn issue_browser_session(state: &AppState, headers: &HeaderMap) -> Response {
    let Some(secret) = browser_session_secret(state) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let (session, expires) = match create_browser_session_cookie(secret.as_bytes()) {
        Ok(value) => value,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    let mut response = Json(BrowserSessionView {
        authenticated: true,
        mode: "browser_session",
        expires_at_unix_ms: Some(expires),
    })
    .into_response();
    let cookie = session_cookie(
        &session,
        BROWSER_SESSION_TTL_MS / 1000,
        secure_cookie_requested(headers),
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

async fn change_browser_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ChangePasswordInput>,
) -> Response {
    let Some(owner) = &state.owner_auth else {
        return (StatusCode::BAD_REQUEST, "Password login is not configured.").into_response();
    };
    if input.current_password.len() > 1024 || input.new_password.len() > 1024 {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    if !owner.admit_attempt() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Too many attempts. Try again shortly.",
        )
            .into_response();
    }
    let Ok(permit) = owner.verification_slots.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let owner = owner.clone();
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        owner.change_password(&input.current_password, &input.new_password)
    })
    .await
    {
        Ok(Ok(())) => {
            state.emit("password_changed", None);
            issue_browser_session(&state, &headers)
        }
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn browser_logout(headers: HeaderMap) -> Response {
    if headers
        .get(BROWSER_CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        != Some("1")
    {
        return (StatusCode::FORBIDDEN, "CSRF header required").into_response();
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    let cookie = session_cookie("", 0, secure_cookie_requested(&headers));
    if let Ok(value) = axum::http::HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
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
        device_name: record.display_name.unwrap_or(record.device_name),
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
        connection_state: if record.revoked {
            "revoked"
        } else if live.is_some() {
            "online"
        } else if now_ms().saturating_sub(record.last_seen_unix_ms) < 60_000 {
            "reconnecting"
        } else {
            "offline"
        },
        last_disconnect_reason: record.last_disconnect_reason,
        last_disconnect_at_unix_ms: record.last_disconnect_at_unix_ms,
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
    let device = resolve_device(state, &device)
        .await
        .map_err(dispatch_error)?;
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
        AgentOperation::System(_) => "system",
        AgentOperation::Network(_) => "network",
        AgentOperation::Services(_) => "services",
        AgentOperation::Containers(_) => "containers",
        AgentOperation::Git(_) => "git",
        AgentOperation::Desktop(_) => "desktop",
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
/// Accept stable IDs or unique human-friendly names. Never mistake an unknown name for
/// an offline device. Name matching is case-insensitive only after exact matching.
fn select_device<'a>(
    records: &'a [DeviceRecord],
    selector: &str,
) -> Result<&'a DeviceRecord, DispatchError> {
    let key = selector.trim();
    if key.is_empty() {
        return Err(DispatchError::NotFound(selector.to_owned()));
    }
    if let Some(record) = records.iter().find(|d| d.device_id == key) {
        return if record.revoked {
            Err(DispatchError::Revoked(record.device_name.clone()))
        } else {
            Ok(record)
        };
    }
    let exact: Vec<_> = records
        .iter()
        .filter(|d| d.device_name == key || d.display_name.as_deref() == Some(key))
        .collect();
    let matches: Vec<_> = if exact.is_empty() {
        records
            .iter()
            .filter(|d| {
                d.device_name.eq_ignore_ascii_case(key)
                    || d.display_name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(key))
            })
            .collect()
    } else {
        exact
    };
    match matches.as_slice() {
        [record] if record.revoked => Err(DispatchError::Revoked(record.device_name.clone())),
        [record] => Ok(record),
        [] => Err(DispatchError::NotFound(selector.to_owned())),
        _ => Err(DispatchError::Ambiguous {
            selector: selector.to_owned(),
            matches: matches
                .iter()
                .map(|d| format!("{} ({})", d.device_name, d.device_id))
                .collect(),
        }),
    }
}

async fn resolve_device(state: &AppState, selector: &str) -> Result<String, DispatchError> {
    // Common paths need no registry lookup; the direct online UUID is canonical.
    if state.devices.read().await.contains_key(selector) {
        return Ok(selector.to_owned());
    }
    let records = state
        .store
        .list_devices()
        .map_err(|err| DispatchError::Storage(err.to_string()))?;
    Ok(select_device(&records, selector)?.device_id.clone())
}

fn dispatch_error(error: DispatchError) -> String {
    match error {
        DispatchError::NotFound(name) => format!(
            "device_not_found: no enrolled device matches '{name}'; use devices to see valid names"
        ),
        DispatchError::Ambiguous { selector, matches } => format!(
            "ambiguous_device: '{selector}' matches {}; select a device ID",
            matches.join(", ")
        ),
        DispatchError::Revoked(name) => {
            format!("device_revoked: '{name}' has been revoked and must be re-enrolled")
        }
        DispatchError::Offline(name) => format!(
            "device_offline: '{name}' is enrolled but is not connected; it will reconnect when reachable"
        ),
        DispatchError::Storage(error) => format!("device_registry_error: {error}"),
        DispatchError::ConnectionClosed => {
            "device_connection_closed: device disconnected while sending the request".into()
        }
        DispatchError::ResponseClosed => {
            "device_response_closed: agent response channel closed".into()
        }
    }
}

async fn dispatch_to_device(
    state: &AppState,
    selector: String,
    operation: AgentOperation,
    timeout_ms: u64,
) -> Result<AgentResponse, DispatchError> {
    let device_id = resolve_device(state, &selector).await?;
    let (tx, retire) = {
        let devices = state.devices.read().await;
        devices
            .get(&device_id)
            .map(|d| (d.tx.clone(), d.retire.clone()))
            .ok_or_else(|| DispatchError::Offline(selector.clone()))?
    };
    let id = Uuid::new_v4();
    let request = GatewayToAgent::Request(AgentRequest { id, operation });
    let (reply_tx, reply_rx) = oneshot::channel();
    state.pending.lock().await.insert(id, reply_tx);
    if tx.send(request).await.is_err() {
        state.pending.lock().await.remove(&id);
        let _ = retire.try_send(());
        return Err(DispatchError::ConnectionClosed);
    }
    match tokio::time::timeout(Duration::from_millis(timeout_ms), reply_rx).await {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(_)) => {
            let _ = retire.try_send(());
            Err(DispatchError::ResponseClosed)
        }
        Err(_) => {
            state.pending.lock().await.remove(&id);
            // An unanswered operation may still complete on the agent. Report an
            // uncertain outcome and never replay it. A slow operation alone is
            // not evidence of a dead WebSocket: the independent heartbeat and
            // socket checks own connection liveness. Retiring here made healthy
            // devices appear offline on each tool timeout.
            warn!(device_id=%device_id, request_id=%id, timeout_ms,
                "agent operation response deadline exceeded; keeping live connection");
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
    let (retire_tx, mut retire_rx) = mpsc::channel::<()>(1);
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
                tx: out_tx.clone(),
                retire: retire_tx,
                reconcile: ReconcileSnapshot::default(),
            },
        );
    }
    let _ = state
        .store
        .record_activity(Some(&hello.device_id), "device.connect", "ok", None);
    state.emit("device_online", Some(hello.device_id.clone()));
    info!(device_id=%hello.device_id,generation,platform=%hello.platform,arch=%hello.arch,"agent online");
    let mut heartbeat = tokio::time::interval(AGENT_HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // `interval` ticks immediately once. Consume that first tick so a newly authenticated
    // connection gets a full interval before its first protocol heartbeat.
    heartbeat.tick().await;
    let mut last_inbound = Instant::now();
    let mut disconnect_reason = "connection_closed";
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if last_inbound.elapsed() >= AGENT_HEARTBEAT_TIMEOUT {
                    warn!(device_id=%hello.device_id,generation,"agent heartbeat timed out; retiring stale connection");
                    disconnect_reason = "heartbeat_timeout";
                    break;
                }
                if out_tx.try_send(GatewayToAgent::Ping).is_err() {
                    warn!(device_id=%hello.device_id,generation,"agent heartbeat queue unavailable");
                    disconnect_reason = "heartbeat_queue_full";
                    break;
                }
            },
            _ = retire_rx.recv() => {
                warn!(device_id=%hello.device_id,generation,"agent connection retired after failed control-path health check");
                disconnect_reason = "control_path_timeout";
                break;
            },
            maybe_out=out_rx.recv()=>match maybe_out{Some(message)=>{
                match tokio::time::timeout(Duration::from_secs(5),send_json(&mut socket,&message)).await {
                    Ok(Ok(()))=>{},
                    Ok(Err(e))=>{warn!(device_id=%hello.device_id,error=%e,"failed to send to agent");disconnect_reason="send_error";break},
                    Err(_)=>{warn!(device_id=%hello.device_id,generation,"timed out sending to agent");disconnect_reason="send_timeout";break},
                }
            },None=>break},
            incoming=socket.recv()=>match incoming{
                Some(Ok(Message::Text(text)))=>{
                    last_inbound=Instant::now();
                    match serde_json::from_str::<AgentToGateway>(text.as_str()){
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
                        Ok(AgentToGateway::Pong)=>{}, Ok(_)=>warn!(device_id=%hello.device_id,"unexpected post-auth agent frame"), Err(e)=>warn!(device_id=%hello.device_id,error=%e,"invalid agent frame")
                    }
                },
                Some(Ok(Message::Ping(payload)))=>{last_inbound=Instant::now();if socket.send(Message::Pong(payload)).await.is_err(){break}},
                Some(Ok(Message::Pong(_)))=>{last_inbound=Instant::now();},
                Some(Ok(Message::Close(_)))|None=>{disconnect_reason="socket_closed";break},Some(Ok(_))=>{},Some(Err(e))=>{warn!(device_id=%hello.device_id,error=%e,"agent websocket error");disconnect_reason="socket_error";break}
            }
        }
    }
    let mut devices = state.devices.write().await;
    if devices
        .get(&hello.device_id)
        .is_some_and(|d| d.generation == generation)
    {
        devices.remove(&hello.device_id);
        let _ = state
            .store
            .record_disconnect(&hello.device_id, disconnect_reason);
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

    fn test_device(id: &str, name: &str) -> DeviceRecord {
        DeviceRecord {
            device_id: id.to_owned(),
            device_name: name.to_owned(),
            display_name: None,
            public_key: None,
            platform: "linux".into(),
            arch: "aarch64".into(),
            agent_version: "test".into(),
            boot_id: "boot-test".into(),
            capabilities: vec!["exec".into()],
            metadata: serde_json::json!({}),
            paired_at_unix_ms: 1,
            last_seen_unix_ms: 1,
            revoked: false,
            reconcile: None,
            last_disconnect_reason: None,
            last_disconnect_at_unix_ms: None,
        }
    }

    #[test]
    fn friendly_names_and_ids_resolve_without_confusing_offline_and_unknown() {
        let records = vec![
            test_device("vm-id", "oracle-vm"),
            test_device("mac-id", "BigMac"),
        ];
        assert_eq!(
            select_device(&records, "oracle-vm").unwrap().device_id,
            "vm-id"
        );
        assert_eq!(
            select_device(&records, "BIGMAC").unwrap().device_id,
            "mac-id"
        );
        assert_eq!(select_device(&records, "vm-id").unwrap().device_id, "vm-id");
        assert!(matches!(
            select_device(&records, "unknown"),
            Err(DispatchError::NotFound(_))
        ));
        assert!(matches!(
            select_device(&records, " "),
            Err(DispatchError::NotFound(_))
        ));
    }

    #[test]
    fn duplicate_names_require_an_unambiguous_device_id() {
        let records = vec![test_device("a-id", "Laptop"), test_device("b-id", "Laptop")];
        assert!(matches!(
            select_device(&records, "Laptop"),
            Err(DispatchError::Ambiguous { .. })
        ));
        assert_eq!(select_device(&records, "b-id").unwrap().device_id, "b-id");
    }

    #[test]
    fn revoked_devices_do_not_route() {
        let mut record = test_device("bad-id", "RevokedMac");
        record.revoked = true;
        assert!(matches!(
            select_device(&[record], "RevokedMac"),
            Err(DispatchError::Revoked(_))
        ));
    }
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
    #[test]
    fn browser_session_survives_gateway_state_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("first.db")).unwrap();
        let state = AppState::new(store, None, Some(TEST_TOKEN.into()));
        let (cookie, expires) = create_browser_session_cookie(TEST_TOKEN.as_bytes()).unwrap();
        assert!(expires > now_ms());
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            axum::http::HeaderValue::from_str(&format!("{BROWSER_SESSION_COOKIE}={cookie}"))
                .unwrap(),
        );
        assert!(browser_session_expiry(&state, &headers).is_some());

        let restarted = AppState::new(
            Store::open(dir.path().join("second.db")).unwrap(),
            None,
            Some(TEST_TOKEN.into()),
        );
        assert!(browser_session_expiry(&restarted, &headers).is_some());
        assert!(!bearer_authorized(&restarted, &headers));

        let mut tampered = headers.clone();
        tampered.insert(
            header::COOKIE,
            axum::http::HeaderValue::from_static("infra_session=v1.1.invalid.invalid"),
        );
        assert!(browser_session_expiry(&restarted, &tampered).is_none());
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
    async fn timed_out_operation_does_not_disconnect_healthy_agent() {
        let (_dir, state, addr, server) = spawn_test_gateway().await;
        let (socket, generation) = connect_legacy(addr, "slow-device").await;
        wait_registered(&state, "slow-device").await;
        let (mut sink, mut stream) = socket.split();
        let responder = tokio::spawn(async move {
            let mut requests = 0;
            while let Some(Ok(ClientMessage::Text(text))) = stream.next().await {
                let message: GatewayToAgent = serde_json::from_str(text.as_str()).unwrap();
                if let GatewayToAgent::Request(request) = message {
                    requests += 1;
                    if requests == 1 {
                        // One command never responds, but the agent can process
                        // subsequent requests on its still-healthy connection.
                        continue;
                    }
                    let answer = AgentToGateway::Response(AgentResponse {
                        id: request.id,
                        result: OperationResult::Ok {
                            body: serde_json::json!({"status": "ready"}),
                        },
                    });
                    sink.send(ClientMessage::Text(
                        serde_json::to_string(&answer).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                }
            }
        });
        let first = dispatch_to_device(&state, "slow-device".into(), AgentOperation::Status, 50)
            .await
            .unwrap();
        assert!(matches!(
            first.result,
            OperationResult::OutcomeUnknown { .. }
        ));
        assert!(state.pending.lock().await.is_empty());
        assert_eq!(
            state.devices.read().await["slow-device"].generation,
            generation
        );
        let next = dispatch_to_device(&state, "slow-device".into(), AgentOperation::Status, 1000)
            .await
            .unwrap();
        assert!(matches!(next.result, OperationResult::Ok { .. }));
        assert!(state.pending.lock().await.is_empty());
        assert_eq!(
            state.devices.read().await["slow-device"].generation,
            generation
        );
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
