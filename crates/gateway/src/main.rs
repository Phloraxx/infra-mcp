mod agent_ws;
mod auth;
mod browser_auth;
mod copy;
mod dashboard_http;
mod hermes_bridge;
mod mcp_actions;
mod mcp_result;
mod oauth;
mod oauth_http;
mod pairing;
mod pairing_http;
mod pending;
mod readiness;
mod store;
mod tools;
mod version;

use agent_ws::*;
use browser_auth::*;
use dashboard_http::*;
use oauth_http::*;
use pairing_http::*;
use tools::*;

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
use hermes_bridge::{HermesBridge, HermesInput, HermesMemoryInput};
use hmac::{Hmac, KeyInit, Mac};
use infra_protocol::{
    AgentOperation, AgentRequest, AgentResponse, AgentToGateway, AuthChallenge, ContainerAction,
    ContainerRequest, DEFAULT_CONTROL_FRAME_LIMIT, DesktopAction, DesktopRequest, DeviceAuthStart,
    DeviceHello, ExecOutput, FILE_LIST_PAGINATION_CAPABILITY, FileAction, FileRequest,
    GatewayToAgent, GitAction, GitRequest, HelloAck, JobAction, JobRequest, LegacyDeviceHello,
    MAX_DIRECTORY_ENTRIES, MAX_DIRECTORY_PAGE_SIZE, NetworkAction, NetworkRequest, OperationResult,
    PROTOCOL_VERSION, ProcessAction, ProcessRequest, ReconcileSnapshot, SearchMode, SearchRequest,
    ServiceAction, ServiceRequest, SignedDeviceHello, SystemAction, SystemRequest, TerminalAction,
    TerminalRequest, TransferAction, TransferRequest, signed_auth_message,
};
use mcp_actions::*;
use mcp_result::{ToolError, ToolJson as McpJson};
use pending::PendingRequests;
use rmcp::{
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
use store::{DeviceRecord, RetentionPolicy, Store, StoreError, now_ms};
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    legacy_agent_token: Option<Arc<str>>,
    admin_token: Option<Arc<str>>,
    owner_auth: Option<Arc<auth::OwnerAuth>>,
    oauth: oauth::OAuthState,
    gateway_epoch: Uuid,
    next_generation: Arc<AtomicU64>,
    devices: Arc<RwLock<HashMap<String, DeviceConnection>>>,
    pending: PendingRequests,
    events: broadcast::Sender<DashboardEvent>,
    store: Store,
    hermes: Option<Arc<HermesBridge>>,
    dashboard: DashboardAssets,
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
            pending: PendingRequests::default(),
            events,
            store,
            hermes: HermesBridge::from_env(),
            dashboard: DashboardAssets::embedded(),
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

#[derive(Debug, Serialize)]
struct Health<'a> {
    status: &'a str,
    service: &'a str,
    protocol: u16,
    version: &'static str,
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

#[derive(Debug)]
enum DispatchError {
    NotFound(String),
    Ambiguous {
        selector: String,
        matches: Vec<String>,
    },
    Revoked(String),
    Offline(String),
    Unsupported(String),
    Storage(String),
    ConnectionClosed,
    QueueBusy,
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
    state.dashboard = DashboardAssets::from_env();
    if let Some(root) = state.dashboard.root() {
        info!(
            dashboard_dir = %root.display(),
            "serving dashboard assets from disk with embedded fallback"
        );
    }
    spawn_retention_task(state.clone());
    let app = build_app(state.clone());
    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!(%bind, version = %version::VERSION, commit = %version::COMMIT, gateway_epoch=%state.gateway_epoch, "Infra v1 gateway listening");
    axum::serve(listener, app).await?;
    Ok(())
}

// Audit rows grow forever on a box with a full disk. Sweep once at startup and
// then daily: a gateway that restarts more often than the interval would
// otherwise never trim the table. The pass is shared with any future table that
// needs a retention window (see RetentionPolicy).
fn spawn_retention_task(state: AppState) {
    let policy = RetentionPolicy::from_env();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match state.store.sweep_retention(policy) {
                Ok(sweep) => info!(
                    audit_events_deleted = sweep.audit_events_deleted,
                    retention_days = policy.audit_retention_days,
                    "retention sweep complete"
                ),
                Err(error) => warn!(%error, "retention sweep failed"),
            }
        }
    });
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
        .route("/api/pair/{request_id}", get(owner_pair_status))
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
        .route("/assets/workspace.png", get(dashboard_workspace))
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

async fn health(State(state): State<AppState>) -> Json<Health<'static>> {
    Json(Health {
        status: "ok",
        service: "infra-gateway",
        protocol: PROTOCOL_VERSION,
        version: version::VERSION_FULL,
        gateway_epoch: state.gateway_epoch,
    })
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

#[cfg(test)]
#[cfg(test)]
mod tests;
