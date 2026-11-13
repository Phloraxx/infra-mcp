use super::*;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DashboardEvent {
    pub(crate) kind: &'static str,
    pub(crate) device_id: Option<String>,
    pub(crate) at_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DashboardSession {
    id: Uuid,
    kind: &'static str,
    status: String,
    boot_id: String,
    started_at_unix_ms: u64,
    device_id: String,
    device_name: String,
    device_online: bool,
}

pub(crate) const DASHBOARD_HTML: &str = include_str!("../../../apps/dashboard/index.html");

pub(crate) const DASHBOARD_JS: &str = include_str!("../../../apps/dashboard/app.js");

pub(crate) const DASHBOARD_CSS: &str = include_str!("../../../apps/dashboard/styles.css");

pub(crate) const INSTALL_SH: &str = include_str!("../../../scripts/install.sh");

pub(crate) const INSTALL_PS1: &str = include_str!("../../../scripts/install.ps1");

pub(crate) async fn dashboard_index() -> Response {
    static_response(DASHBOARD_HTML, "text/html; charset=utf-8")
}

pub(crate) async fn dashboard_js() -> Response {
    static_response(DASHBOARD_JS, "text/javascript; charset=utf-8")
}

pub(crate) async fn dashboard_css() -> Response {
    static_response(DASHBOARD_CSS, "text/css; charset=utf-8")
}

pub(crate) async fn dashboard_logo() -> Response {
    image_response(
        include_bytes!("../../../apps/dashboard/assets/logo.png"),
        "image/png",
    )
}

pub(crate) async fn dashboard_workspace() -> Response {
    image_response(
        include_bytes!("../../../apps/dashboard/assets/workspace.png"),
        "image/png",
    )
}

pub(crate) async fn dashboard_favicon() -> Response {
    image_response(
        include_bytes!("../../../apps/dashboard/assets/favicon.png"),
        "image/png",
    )
}

pub(crate) async fn dashboard_favicon_ico() -> Response {
    image_response(
        include_bytes!("../../../apps/dashboard/assets/favicon.ico"),
        "image/x-icon",
    )
}

pub(crate) async fn dashboard_touch_icon() -> Response {
    image_response(
        include_bytes!("../../../apps/dashboard/assets/apple-touch-icon.png"),
        "image/png",
    )
}

pub(crate) fn image_response(data: &'static [u8], content_type: &'static str) -> Response {
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

pub(crate) async fn dashboard_edge(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
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

pub(crate) async fn dashboard_terminal_js() -> Response {
    static_response(
        include_str!("../../../apps/dashboard/vendor/xterm.js"),
        "text/javascript; charset=utf-8",
    )
}

pub(crate) async fn dashboard_terminal_css() -> Response {
    static_response(
        include_str!("../../../apps/dashboard/vendor/xterm.css"),
        "text/css; charset=utf-8",
    )
}

pub(crate) async fn download_install_sh() -> Response {
    static_response(INSTALL_SH, "text/x-shellscript; charset=utf-8")
}

pub(crate) async fn download_install_ps1() -> Response {
    static_response(INSTALL_PS1, "text/plain; charset=utf-8")
}

pub(crate) fn static_response(body: &'static str, content_type: &'static str) -> Response {
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

pub(crate) async fn dashboard_events(
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

pub(crate) async fn list_sessions_http(State(state): State<AppState>) -> Response {
    match dashboard_sessions(&state).await {
        Ok(sessions) => Json(sessions).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(crate) async fn list_activity_http(State(state): State<AppState>) -> Response {
    match state.store.list_activity(100) {
        Ok(activity) => Json(activity).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

// The browser and MCP use the same typed operations, dispatcher, durable workers
// and audit path. Keep the browser surface limited to the implemented controls.
pub(crate) async fn dashboard_operation(
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
                action: JobAction::List
                    | JobAction::Read { .. }
                    | JobAction::Wait { .. }
                    | JobAction::Cancel { .. }
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
                Json(serde_json::json!({"error":error.message,"error_code":error.code})),
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

pub(crate) async fn dashboard_sessions(
    state: &AppState,
) -> Result<Vec<DashboardSession>, StoreError> {
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

// Single bounded snapshot avoids three independent HTTP round trips on each UI refresh.
pub(crate) async fn overview_http(State(state): State<AppState>) -> Response {
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

pub(crate) async fn list_devices_http(State(state): State<AppState>) -> Response {
    match device_views(&state).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
