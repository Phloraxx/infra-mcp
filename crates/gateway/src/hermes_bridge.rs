//! Optional, local-only Hermes integration. No Hermes dependency in the device agent.
//! API calls use Hermes' authenticated Runs/Sessions API; memory mutations run through
//! Hermes' own MemoryStore to preserve its locking, validation and atomic updates.
use crate::mcp_result::ToolError;
use reqwest::{Client, Method};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{env, path::PathBuf, sync::Arc, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};
use url::Url;

const MAX_API_RESPONSE: usize = 256 * 1024;
const MAX_MEMORY_RESPONSE: usize = 96 * 1024;
const MEMORY_HELPER: &str = include_str!("hermes_memory_helper.py");

#[derive(Clone)]
pub struct HermesBridge {
    client: Client,
    base: Url,
    key_file: PathBuf,
    key_in_env_file: bool,
    home: PathBuf,
    source: PathBuf,
    python: PathBuf,
    memory_writes: bool,
    runs_enabled: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HermesInput {
    /// capabilities, start, status, stop, sessions, messages or search
    pub action: String,
    /// Required for start. Describe a specific bounded task for Hermes.
    pub input: Option<String>,
    /// Stable client-generated ID required for start; reused attempts are idempotent.
    pub request_id: Option<String>,
    /// Returned Hermes run ID (required for status/stop).
    pub run_id: Option<String>,
    /// API-owned session ID for messages; optional on start. Not a CLI/messaging session.
    pub session_id: Option<String>,
    /// Page size for sessions, max 50.
    pub limit: Option<u32>,
    /// Sessions page offset, max 10000.
    pub offset: Option<u32>,
    /// Search prior Hermes sessions by keyword; 1..512 bytes.
    pub query: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HermesMemoryInput {
    /// read, add, patch, or remove. Writes require gateway operator opt-in.
    pub action: String,
    /// memory (agent notes) or user (stable user profile).
    pub target: String,
    /// New text for add or patch (max 2200 characters).
    pub content: Option<String>,
    /// Existing exact/unique text for patch/remove (max 2200 characters).
    pub old_text: Option<String>,
}

fn unavailable(detail: &'static str) -> ToolError {
    ToolError::new("unsupported", detail)
}
fn invalid(msg: &'static str) -> ToolError {
    ToolError::invalid(msg)
}
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 160
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b':'))
}
fn valid_request_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

impl HermesBridge {
    pub fn from_env() -> Option<Arc<Self>> {
        if env::var("INFRA_HERMES_ENABLED").as_deref() != Ok("1") {
            return None;
        }
        let result = (|| {
            let raw = env::var("INFRA_HERMES_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8642/".into());
            let base = Url::parse(&raw).ok()?;
            // No configurable remote URLs: Hermes full-agent API is extremely privileged.
            if base.scheme() != "http"
                || !matches!(base.host_str(), Some("127.0.0.1" | "::1"))
                || base.port().is_none_or(|p| p == 0)
                || base.username() != ""
                || base.password().is_some()
                || base.query().is_some()
                || base.fragment().is_some()
                || base.path() != "/"
            {
                return None;
            }
            let (key_file, key_in_env_file) = match (
                env::var_os("INFRA_HERMES_API_KEY_FILE"),
                env::var_os("INFRA_HERMES_API_ENV_FILE"),
            ) {
                (Some(p), None) => (PathBuf::from(p), false),
                (None, Some(p)) => (PathBuf::from(p), true),
                _ => return None, // Exactly one credential source.
            };
            let home = PathBuf::from(env::var_os("INFRA_HERMES_HOME")?);
            let source = PathBuf::from(env::var_os("INFRA_HERMES_SOURCE")?);
            let python = PathBuf::from(env::var_os("INFRA_HERMES_PYTHON")?);
            if ![&key_file, &home, &source, &python]
                .iter()
                .all(|p| p.is_absolute())
            {
                return None;
            }
            let client = Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .build()
                .ok()?;
            Some(Self {
                client,
                base,
                key_file,
                key_in_env_file,
                home,
                source,
                python,
                memory_writes: env::var("INFRA_HERMES_MEMORY_WRITES").as_deref() == Ok("1"),
                runs_enabled: env::var("INFRA_HERMES_RUNS_ENABLED").as_deref() == Ok("1"),
            })
        })();
        if result.is_none() {
            tracing::warn!("Hermes bridge configuration invalid; bridge disabled");
        }
        result.map(Arc::new)
    }

    async fn api(
        &self,
        method: Method,
        route: &str,
        body: Option<Value>,
        idempotency: Option<&str>,
    ) -> Result<Value, ToolError> {
        // Route is always an authored relative route, never a user-supplied URL.
        let url = self
            .base
            .join(route)
            .map_err(|_| invalid("Invalid Hermes API route"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = tokio::fs::metadata(&self.key_file)
                .await
                .map_err(|_| unavailable("Hermes bridge key file unreadable"))?;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(unavailable(
                    "Hermes bridge key file must have mode 0600 or stricter",
                ));
            }
        }
        let raw = tokio::fs::read_to_string(&self.key_file)
            .await
            .map_err(|_| unavailable("Hermes bridge key file unreadable"))?;
        let value = if self.key_in_env_file {
            let candidates: Vec<_> = raw
                .lines()
                .filter_map(|line| {
                    line.trim_start()
                        .strip_prefix("API_SERVER_KEY=")
                        .map(str::trim)
                })
                .collect();
            if candidates.len() != 1 {
                return Err(unavailable(
                    "Hermes env file must contain exactly one API_SERVER_KEY",
                ));
            }
            let value = candidates[0];
            if value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')))
            {
                value[1..value.len() - 1].to_owned()
            } else {
                value.to_owned()
            }
        } else {
            raw.trim().to_owned()
        };
        let key = value.as_str();
        if key.is_empty() || key.contains(['\n', '\r']) || key.len() > 1024 {
            return Err(unavailable("Hermes bridge key missing or malformed"));
        }
        let mut request = self.client.request(method, url).bearer_auth(key);
        if let Some(v) = body {
            request = request.json(&v);
        }
        if let Some(id) = idempotency {
            request = request.header("Idempotency-Key", id);
        }
        let response = request.send().await
            .map_err(|_| ToolError::new("operation_failed", "Hermes API unreachable or timed out; do not replay an uncertain start with a different request_id"))?;
        let status = response.status();
        let mut response = response;
        if response
            .content_length()
            .is_some_and(|n| n > MAX_API_RESPONSE as u64)
        {
            return Err(invalid("Hermes response exceeds configured size limit"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ToolError::new("operation_failed", "Could not read Hermes response"))?
        {
            if bytes.len() + chunk.len() > MAX_API_RESPONSE {
                return Err(invalid("Hermes response exceeds configured size limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            // Never surface upstream error bodies, which can contain paths or data.
            return Err(ToolError::new(
                "operation_failed",
                format!("Hermes API returned HTTP {status}"),
            ));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| ToolError::new("operation_failed", "Hermes returned malformed JSON"))
    }

    pub async fn run(&self, input: HermesInput) -> Result<Value, ToolError> {
        match input.action.as_str() {
            "capabilities" => self.api(Method::GET, "v1/capabilities", None, None).await,
            "start" => {
                if !self.runs_enabled {
                    return Err(unavailable(
                        "Hermes task delegation disabled by gateway operator",
                    ));
                }
                let message = input
                    .input
                    .as_deref()
                    .ok_or_else(|| invalid("input is required for start"))?;
                if message.trim().is_empty() || message.len() > 12_000 {
                    return Err(invalid("input must be 1..12000 bytes"));
                }
                let request_id = input
                    .request_id
                    .as_deref()
                    .ok_or_else(|| invalid("request_id is required for safe retry of start"))?;
                if !valid_request_id(request_id) {
                    return Err(invalid(
                        "request_id must be 1..128 ASCII characters: letters, digits, '-', '_' or '.'",
                    ));
                }
                let mut body = json!({"input": message});
                if let Some(session_id) = input.session_id {
                    if !valid_id(&session_id) {
                        return Err(invalid("Invalid API-owned session_id"));
                    }
                    body["session_id"] = json!(session_id);
                }
                self.api(Method::POST, "v1/runs", Some(body), Some(request_id))
                    .await
            }
            "status" | "stop" => {
                if input.action == "stop" && !self.runs_enabled {
                    return Err(unavailable(
                        "Hermes task delegation disabled by gateway operator",
                    ));
                }
                let run_id = input.run_id.ok_or_else(|| invalid("run_id is required"))?;
                if !run_id.starts_with("run_") || !valid_id(&run_id) {
                    return Err(invalid("Invalid Hermes run_id"));
                }
                let route = if input.action == "stop" {
                    format!("v1/runs/{run_id}/stop")
                } else {
                    format!("v1/runs/{run_id}")
                };
                self.api(
                    if input.action == "stop" {
                        Method::POST
                    } else {
                        Method::GET
                    },
                    &route,
                    None,
                    None,
                )
                .await
            }
            "search" => {
                let query = input
                    .query
                    .as_deref()
                    .ok_or_else(|| invalid("query is required for search"))?;
                if query.trim().is_empty() || query.len() > 512 {
                    return Err(invalid("query must be 1..512 bytes"));
                }
                self.python_helper(json!({"action": "search_history", "target": "memory",
                    "query": query, "limit": input.limit.unwrap_or(3).clamp(1,10)}))
                    .await
            }
            "sessions" => {
                let limit = input.limit.unwrap_or(10).clamp(1, 50);
                let offset = input.offset.unwrap_or(0).min(10000);
                self.api(
                    Method::GET,
                    &format!("api/sessions?limit={limit}&offset={offset}"),
                    None,
                    None,
                )
                .await
            }
            "messages" => {
                let session_id = input
                    .session_id
                    .ok_or_else(|| invalid("session_id is required"))?;
                if !valid_id(&session_id) {
                    return Err(invalid("Invalid session_id"));
                }
                self.api(
                    Method::GET,
                    &format!("api/sessions/{session_id}/messages"),
                    None,
                    None,
                )
                .await
            }
            _ => Err(invalid(
                "action must be capabilities, start, status, stop, sessions, messages or search",
            )),
        }
    }

    pub async fn memory(&self, input: HermesMemoryInput) -> Result<Value, ToolError> {
        if !matches!(input.target.as_str(), "user" | "memory") {
            return Err(invalid("target must be user or memory"));
        }
        if !matches!(input.action.as_str(), "read" | "add" | "patch" | "remove") {
            return Err(invalid("action must be read, add, patch, or remove"));
        }
        if input.action != "read" && !self.memory_writes {
            return Err(unavailable(
                "Hermes memory writes are disabled by gateway operator",
            ));
        }
        if matches!(input.action.as_str(), "add" | "patch")
            && !input
                .content
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty() && s.len() <= 2200)
        {
            return Err(invalid(
                "content is required and must be at most 2200 bytes",
            ));
        }
        if matches!(input.action.as_str(), "patch" | "remove")
            && !input
                .old_text
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty() && s.len() <= 2200)
        {
            return Err(invalid(
                "old_text is required and must be at most 2200 bytes",
            ));
        }
        let payload = json!({"action": input.action, "target": input.target,
            "content": input.content, "old_text": input.old_text});
        self.python_helper(payload).await
    }

    async fn python_helper(&self, payload: Value) -> Result<Value, ToolError> {
        let mut process = Command::new(&self.python);
        process
            .arg("-I")
            .arg("-c")
            .arg(MEMORY_HELPER)
            .env("HERMES_HOME", &self.home)
            .env("INFRA_HERMES_SOURCE", &self.source)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = process
            .spawn()
            .map_err(|_| unavailable("Hermes Python environment unavailable"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(payload.to_string().as_bytes())
                .await
                .map_err(|_| {
                    ToolError::new("operation_failed", "Could not send memory operation")
                })?;
        }
        let output = tokio::time::timeout(Duration::from_secs(15), child.wait_with_output())
            .await
            .map_err(|_| {
                ToolError::new(
                    "deadline_exceeded",
                    "Memory operation exceeded 15 seconds; inspect memory before retrying",
                )
            })?
            .map_err(|_| ToolError::new("operation_failed", "Memory helper failed"))?;
        if output.stdout.len() > MAX_MEMORY_RESPONSE {
            return Err(invalid("Memory helper response too large"));
        }
        let value: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
            ToolError::new(
                "operation_failed",
                "Hermes memory helper returned invalid JSON",
            )
        })?;
        if !output.status.success() || value.get("success") == Some(&Value::Bool(false)) {
            return Err(ToolError::new(
                "operation_failed",
                format!(
                    "Hermes refused the memory operation: {}",
                    value
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                ),
            ));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inputs_do_not_allow_path_injection() {
        for s in [
            "",
            "../sessions",
            "a/b",
            "a?limit=10000",
            "a#fragment",
            "a\nkey: x",
        ] {
            assert!(!valid_id(s), "{s}");
            assert!(!valid_request_id(s), "{s}");
        }
        assert!(valid_id("run_ab-123"));
        assert!(valid_id("local-abc123"));
    }
    #[tokio::test]
    async fn mock_run_api_preserves_idempotency_without_exec() {
        use axum::{Json, Router, http::HeaderMap, routing::post};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let route = Router::new().route(
            "/v1/runs",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let calls = Arc::clone(&calls_clone);
                async move {
                    assert_eq!(headers.get("idempotency-key").unwrap(), "client-123");
                    assert_eq!(
                        headers.get("authorization").unwrap(),
                        "Bearer fake-test-key"
                    );
                    assert_eq!(body["input"], "Diagnose a test.");
                    calls.fetch_add(1, Ordering::Relaxed);
                    Json(json!({"run_id":"run_fixture","status":"started"}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, route).await.unwrap();
        });
        let scratch = tempfile::tempdir().unwrap();
        let key = scratch.path().join("api.key");
        std::fs::write(&key, "fake-test-key\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let bridge = HermesBridge {
            client: Client::new(),
            base: Url::parse(&format!("http://{addr}/")).unwrap(),
            key_file: key,
            key_in_env_file: false,
            home: scratch.path().to_owned(),
            source: scratch.path().to_owned(),
            python: scratch.path().join("unused"),
            memory_writes: false,
            runs_enabled: true,
        };
        let response = bridge
            .run(HermesInput {
                action: "start".into(),
                input: Some("Diagnose a test.".into()),
                request_id: Some("client-123".into()),
                run_id: None,
                session_id: None,
                limit: None,
                offset: None,
                query: None,
            })
            .await
            .unwrap();
        assert_eq!(response["run_id"], "run_fixture");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let read_only = HermesBridge {
            runs_enabled: false,
            ..bridge.clone()
        };
        let denied_run = read_only
            .run(HermesInput {
                action: "start".into(),
                input: Some("Must never run.".into()),
                request_id: Some("client-124".into()),
                run_id: None,
                session_id: None,
                limit: None,
                offset: None,
                query: None,
            })
            .await
            .unwrap_err();
        assert!(format!("{denied_run:?}").contains("disabled"));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let denied = bridge
            .memory(HermesMemoryInput {
                action: "add".into(),
                target: "memory".into(),
                content: Some("should never be written".into()),
                old_text: None,
            })
            .await
            .unwrap_err();
        assert!(format!("{denied:?}").contains("disabled"));
        server.abort();
    }
}
