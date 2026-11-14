use super::*;
use sha2::Digest;
use std::fs;

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

pub(crate) const DASHBOARD_TERMINAL_JS: &str =
    include_str!("../../../apps/dashboard/vendor/xterm.js");

pub(crate) const DASHBOARD_TERMINAL_CSS: &str =
    include_str!("../../../apps/dashboard/vendor/xterm.css");

pub(crate) const INSTALL_SH: &str = include_str!("../../../scripts/install.sh");

pub(crate) const INSTALL_PS1: &str = include_str!("../../../scripts/install.ps1");

const DASHBOARD_CSP: &str = "default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

// A dashboard asset is a few hundred kilobytes at most. Anything larger is a
// misconfigured override rather than a UI file, so fall back to the embedded copy.
const MAX_DASHBOARD_ASSET_BYTES: u64 = 8 * 1024 * 1024;

/// Optional on-disk dashboard assets.
///
/// The dashboard is compiled into the binary so a gateway is self-contained.
/// `INFRA_DASHBOARD_DIR` is an override for iterating on the UI without a
/// rebuild and the restart that drops every agent link. Every lookup falls back
/// to the embedded bytes when the directory, the file, or the read is unusable,
/// so a broken override degrades to the shipped behaviour instead of a broken UI.
#[derive(Clone, Debug, Default)]
pub(crate) struct DashboardAssets {
    root: Option<Arc<PathBuf>>,
}

impl DashboardAssets {
    pub(crate) fn embedded() -> Self {
        Self::at(None)
    }

    pub(crate) fn from_env() -> Self {
        Self::at(
            std::env::var("INFRA_DASHBOARD_DIR")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
        )
    }

    pub(crate) fn at(root: Option<PathBuf>) -> Self {
        Self {
            root: root.map(Arc::new),
        }
    }

    pub(crate) fn root(&self) -> Option<&std::path::Path> {
        self.root.as_deref().map(PathBuf::as_path)
    }

    fn read(&self, relative: &str) -> Option<Vec<u8>> {
        let path = self.root.as_deref()?.join(relative);
        let metadata = fs::metadata(&path).ok()?;
        if !metadata.is_file() || metadata.len() > MAX_DASHBOARD_ASSET_BYTES {
            return None;
        }
        fs::read(path).ok()
    }
}

pub(crate) async fn dashboard_index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    dashboard_asset(
        &state.dashboard,
        "index.html",
        DASHBOARD_HTML,
        "text/html; charset=utf-8",
        &headers,
    )
}

pub(crate) async fn dashboard_js(State(state): State<AppState>, headers: HeaderMap) -> Response {
    dashboard_asset(
        &state.dashboard,
        "app.js",
        DASHBOARD_JS,
        "text/javascript; charset=utf-8",
        &headers,
    )
}

pub(crate) async fn dashboard_css(State(state): State<AppState>, headers: HeaderMap) -> Response {
    dashboard_asset(
        &state.dashboard,
        "styles.css",
        DASHBOARD_CSS,
        "text/css; charset=utf-8",
        &headers,
    )
}

// Disk-served assets carry a content-hash ETag and `no-cache` so a browser
// revalidates on every load: an edited file shows up on refresh, and an
// unchanged one costs a 304 instead of a body. The embedded path keeps the
// original `no-store` response byte for byte.
fn dashboard_asset(
    assets: &DashboardAssets,
    relative: &str,
    embedded: &'static str,
    content_type: &'static str,
    headers: &HeaderMap,
) -> Response {
    let Some(body) = assets.read(relative) else {
        return static_response(embedded, content_type);
    };
    let etag = format!(
        "\"{}\"",
        URL_SAFE_NO_PAD.encode(&Sha256::digest(&body)[..16])
    );
    let revalidated = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|candidate| candidate.trim().trim_start_matches("W/") == etag)
        });
    if revalidated {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CACHE_CONTROL, "no-cache".to_owned()),
            ],
        )
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, content_type.to_owned()),
            (header::CACHE_CONTROL, "no-cache".to_owned()),
            (header::ETAG, etag),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
            (header::CONTENT_SECURITY_POLICY, DASHBOARD_CSP.to_owned()),
        ],
        body,
    )
        .into_response()
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
    // Default every response to `no-store`; a handler that already chose a
    // cache policy (the disk-served dashboard assets use `no-cache` plus an
    // ETag) keeps it.
    if !response.headers().contains_key(header::CACHE_CONTROL) {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
    }
    response
}

pub(crate) async fn dashboard_terminal_js(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    dashboard_asset(
        &state.dashboard,
        "vendor/xterm.js",
        DASHBOARD_TERMINAL_JS,
        "text/javascript; charset=utf-8",
        &headers,
    )
}

pub(crate) async fn dashboard_terminal_css(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    dashboard_asset(
        &state.dashboard,
        "vendor/xterm.css",
        DASHBOARD_TERMINAL_CSS,
        "text/css; charset=utf-8",
        &headers,
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
            // xterm renders its grid using generated styles. Script execution
            // remains self-only; no remote assets or inline scripts are allowed.
            (header::CONTENT_SECURITY_POLICY, DASHBOARD_CSP),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    async fn spawn_asset_gateway(
        assets: DashboardAssets,
    ) -> (TempDir, SocketAddr, tokio::task::JoinHandle<()>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("infra.db")).unwrap();
        let mut state = AppState::new(store, None, None);
        state.dashboard = assets;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = build_app(state);
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (dir, addr, task)
    }

    fn header_value(response: &reqwest::Response, name: header::HeaderName) -> Option<String> {
        response
            .headers()
            .get(name)
            .map(|value| value.to_str().unwrap().to_owned())
    }

    const ASSETS: [(&str, &str, &str); 5] = [
        ("/", "index.html", "text/html; charset=utf-8"),
        ("/app.js", "app.js", "text/javascript; charset=utf-8"),
        ("/styles.css", "styles.css", "text/css; charset=utf-8"),
        (
            "/vendor/xterm.js",
            "vendor/xterm.js",
            "text/javascript; charset=utf-8",
        ),
        (
            "/vendor/xterm.css",
            "vendor/xterm.css",
            "text/css; charset=utf-8",
        ),
    ];

    fn embedded_body(relative: &str) -> &'static str {
        match relative {
            "index.html" => DASHBOARD_HTML,
            "app.js" => DASHBOARD_JS,
            "styles.css" => DASHBOARD_CSS,
            "vendor/xterm.js" => DASHBOARD_TERMINAL_JS,
            "vendor/xterm.css" => DASHBOARD_TERMINAL_CSS,
            other => panic!("unmapped asset {other}"),
        }
    }

    fn write_asset(root: &Path, relative: &str, body: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[tokio::test]
    async fn embedded_assets_are_served_byte_for_byte_without_an_override() {
        let (_dir, addr, server) = spawn_asset_gateway(DashboardAssets::embedded()).await;
        let client = reqwest::Client::new();
        for (route, relative, content_type) in ASSETS {
            let response = client
                .get(format!("http://{addr}{route}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200, "{route}");
            assert_eq!(
                header_value(&response, header::CONTENT_TYPE).as_deref(),
                Some(content_type),
                "{route}"
            );
            assert_eq!(
                header_value(&response, header::CACHE_CONTROL).as_deref(),
                Some("no-store"),
                "{route}"
            );
            assert!(
                header_value(&response, header::ETAG).is_none(),
                "{route} must not advertise an ETag without an override"
            );
            assert_eq!(response.text().await.unwrap(), embedded_body(relative));
        }
        server.abort();
    }

    #[tokio::test]
    async fn disk_override_serves_files_with_a_content_hash_etag() {
        let assets = tempfile::tempdir().unwrap();
        for (_, relative, _) in ASSETS {
            write_asset(assets.path(), relative, &format!("/* disk {relative} */"));
        }
        let (_dir, addr, server) =
            spawn_asset_gateway(DashboardAssets::at(Some(assets.path().to_path_buf()))).await;
        let client = reqwest::Client::new();
        for (route, relative, content_type) in ASSETS {
            let response = client
                .get(format!("http://{addr}{route}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200, "{route}");
            assert_eq!(
                header_value(&response, header::CONTENT_TYPE).as_deref(),
                Some(content_type),
                "{route}"
            );
            assert_eq!(
                header_value(&response, header::CACHE_CONTROL).as_deref(),
                Some("no-cache"),
                "{route}"
            );
            let etag = header_value(&response, header::ETAG).expect("disk assets carry an ETag");
            assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");
            assert_eq!(
                response.text().await.unwrap(),
                format!("/* disk {relative} */")
            );
        }
        server.abort();
    }

    #[tokio::test]
    async fn disk_override_revalidates_with_if_none_match_and_tracks_content_changes() {
        let assets = tempfile::tempdir().unwrap();
        write_asset(assets.path(), "styles.css", "body{color:#fff}");
        let (_dir, addr, server) =
            spawn_asset_gateway(DashboardAssets::at(Some(assets.path().to_path_buf()))).await;
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/styles.css");
        let first = client.get(&url).send().await.unwrap();
        let etag = header_value(&first, header::ETAG).unwrap();
        assert_eq!(first.text().await.unwrap(), "body{color:#fff}");

        let revalidated = client
            .get(&url)
            .header(header::IF_NONE_MATCH, &etag)
            .send()
            .await
            .unwrap();
        assert_eq!(revalidated.status(), 304);
        assert_eq!(
            header_value(&revalidated, header::CACHE_CONTROL).as_deref(),
            Some("no-cache")
        );
        assert_eq!(revalidated.text().await.unwrap(), "");

        // The ETag is a content hash, so an edit invalidates it even when the
        // file size and modification time are unchanged.
        write_asset(assets.path(), "styles.css", "body{color:#000}");
        let changed = client
            .get(&url)
            .header(header::IF_NONE_MATCH, &etag)
            .send()
            .await
            .unwrap();
        assert_eq!(changed.status(), 200);
        assert_ne!(header_value(&changed, header::ETAG).unwrap(), etag);
        assert_eq!(changed.text().await.unwrap(), "body{color:#000}");
        server.abort();
    }

    #[tokio::test]
    async fn a_missing_or_partial_override_falls_back_to_the_embedded_bytes() {
        let missing = tempfile::tempdir().unwrap().path().join("absent");
        let (_dir, addr, server) = spawn_asset_gateway(DashboardAssets::at(Some(missing))).await;
        let client = reqwest::Client::new();
        let response = client.get(format!("http://{addr}/")).send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            header_value(&response, header::CACHE_CONTROL).as_deref(),
            Some("no-store")
        );
        assert!(header_value(&response, header::ETAG).is_none());
        assert_eq!(response.text().await.unwrap(), DASHBOARD_HTML);
        server.abort();

        // A directory that only holds some of the files serves those from disk
        // and the rest from the binary.
        let partial = tempfile::tempdir().unwrap();
        write_asset(
            partial.path(),
            "index.html",
            "<!doctype html><title>disk</title>",
        );
        let (_dir, addr, server) =
            spawn_asset_gateway(DashboardAssets::at(Some(partial.path().to_path_buf()))).await;
        let index = client.get(format!("http://{addr}/")).send().await.unwrap();
        assert_eq!(
            header_value(&index, header::CACHE_CONTROL).as_deref(),
            Some("no-cache")
        );
        assert_eq!(
            index.text().await.unwrap(),
            "<!doctype html><title>disk</title>"
        );
        let script = client
            .get(format!("http://{addr}/app.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            header_value(&script, header::CACHE_CONTROL).as_deref(),
            Some("no-store")
        );
        assert_eq!(script.text().await.unwrap(), DASHBOARD_JS);
        server.abort();
    }

    #[tokio::test]
    async fn a_directory_where_a_file_belongs_falls_back_to_the_embedded_bytes() {
        let assets = tempfile::tempdir().unwrap();
        fs::create_dir_all(assets.path().join("app.js")).unwrap();
        let (_dir, addr, server) =
            spawn_asset_gateway(DashboardAssets::at(Some(assets.path().to_path_buf()))).await;
        let response = reqwest::Client::new()
            .get(format!("http://{addr}/app.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            header_value(&response, header::CACHE_CONTROL).as_deref(),
            Some("no-store")
        );
        assert_eq!(response.text().await.unwrap(), DASHBOARD_JS);
        server.abort();
    }

    #[test]
    fn the_override_root_is_resolved_from_the_environment_only_when_set() {
        // `from_env` is the only reader of INFRA_DASHBOARD_DIR; the default
        // state must stay embedded so an unset variable cannot change output.
        assert!(DashboardAssets::embedded().root().is_none());
        assert!(DashboardAssets::at(None).root().is_none());
        assert_eq!(
            DashboardAssets::at(Some(PathBuf::from("/tmp/infra-dashboard")))
                .root()
                .unwrap(),
            Path::new("/tmp/infra-dashboard")
        );
    }
}
