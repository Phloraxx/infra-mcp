//! Optional, local-only Mnemosyne memory integration for the infra gateway.
//! Proxies curated Mnemosyne MCP memory operations over loopback streamable
//! HTTP using the operator-configured per-agent bearer token, so every
//! memory written through this bridge is attributed to this gateway agent
//! identity. Each tool call performs a fresh MCP handshake, keeping the
//! bridge stateless and restart-safe. No Mnemosyne dependency in the device
//! agent.
use crate::mcp_result::ToolError;
use reqwest::Client;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{env, sync::Arc, time::Duration};
use url::Url;

const MAX_CONTENT_CHARS: usize = 8_000;
const MAX_QUERY_CHARS: usize = 1_000;
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Clone)]
pub struct MnemosyneBridge {
    client: Client,
    base: Url,
    token: Arc<str>,
    allow_sleep: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryInput {
    /// remember, recall, get, update, invalidate, forget, stats or sleep.
    pub action: String,
    /// remember: memory content to store; update: replacement content.
    pub content: Option<String>,
    /// recall: natural-language query.
    pub query: Option<String>,
    /// recall: maximum results, 1..=20. Default 5.
    pub limit: Option<u32>,
    /// get/update/invalidate/forget: existing memory ID.
    pub memory_id: Option<String>,
    /// remember/update: importance 0.0-1.0. Default 0.5.
    pub importance: Option<f64>,
    /// remember: source tag such as preference, fact, insight, identity or task.
    pub source: Option<String>,
    /// remember: optional expiry date YYYY-MM-DD for time-bound facts.
    pub valid_until: Option<String>,
}

fn unavailable(detail: &str) -> ToolError {
    ToolError::new("unsupported", detail)
}
fn invalid(msg: &'static str) -> ToolError {
    ToolError::invalid(msg)
}
fn valid_memory_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}

impl MnemosyneBridge {
    /// Build from environment variables. Returns None when disabled or
    /// misconfigured. Loopback-only URLs are enforced: memory data never
    /// leaves the host.
    pub fn from_env() -> Option<Arc<Self>> {
        if env::var("INFRA_MNEMOSYNE_ENABLED").as_deref() != Ok("1") {
            return None;
        }
        (|| {
            let raw = env::var("INFRA_MNEMOSYNE_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080/mcp".into());
            let base = Url::parse(&raw).ok()?;
            if base.scheme() != "http"
                || !matches!(
                    base.host_str(),
                    Some("127.0.0.1") | Some("::1") | Some("localhost")
                )
                || base.username() != ""
                || base.password().is_some()
                || base.query().is_some()
                || base.fragment().is_some()
            {
                return None;
            }
            let token = env::var("INFRA_MNEMOSYNE_TOKEN").ok()?;
            if !(16..=512).contains(&token.len()) {
                return None;
            }
            let allow_sleep = env::var("INFRA_MNEMOSYNE_ALLOW_SLEEP").as_deref() == Ok("1");
            let client = Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(120))
                .build()
                .ok()?;
            Some(Arc::new(Self {
                client,
                base,
                token: token.into(),
                allow_sleep,
            }))
        })()
    }

    pub async fn run(&self, input: MemoryInput) -> Result<Value, ToolError> {
        match input.action.as_str() {
            "remember" => self.remember(input).await,
            "recall" => self.recall(input).await,
            "get" => self.with_id(input, "mnemosyne_get").await,
            "forget" => self.with_id(input, "mnemosyne_forget").await,
            "invalidate" => self.with_id(input, "mnemosyne_invalidate").await,
            "update" => self.update(input).await,
            "stats" => self.call_tool("mnemosyne_stats", json!({})).await,
            "sleep" => {
                if !self.allow_sleep {
                    return Err(unavailable(
                        "Sleep consolidation is disabled by the operator (INFRA_MNEMOSYNE_ALLOW_SLEEP)",
                    ));
                }
                self.call_tool("mnemosyne_sleep", json!({})).await
            }
            _ => Err(invalid(
                "action must be one of remember, recall, get, update, invalidate, forget, stats, sleep",
            )),
        }
    }

    fn checked_content(input: &MemoryInput) -> Result<&str, ToolError> {
        let content = input
            .content
            .as_deref()
            .ok_or_else(|| invalid("content is required"))?;
        if content.is_empty() || content.chars().count() > MAX_CONTENT_CHARS {
            return Err(invalid("content must be 1..=8000 characters"));
        }
        Ok(content)
    }

    fn checked_id(input: &MemoryInput) -> Result<&str, ToolError> {
        let id = input
            .memory_id
            .as_deref()
            .ok_or_else(|| invalid("memory_id is required"))?;
        if !valid_memory_id(id) {
            return Err(invalid("memory_id is malformed"));
        }
        Ok(id)
    }

    async fn remember(&self, input: MemoryInput) -> Result<Value, ToolError> {
        let content = Self::checked_content(&input)?;
        let mut args = json!({ "content": content });
        if let Some(importance) = input.importance {
            if !(0.0..=1.0).contains(&importance) {
                return Err(invalid("importance must be between 0.0 and 1.0"));
            }
            args["importance"] = json!(importance);
        }
        if let Some(source) = input.source.as_deref() {
            if source.is_empty() || source.len() > 64 {
                return Err(invalid("source must be 1..=64 characters"));
            }
            args["source"] = json!(source);
        }
        if let Some(valid_until) = input.valid_until.as_deref() {
            if valid_until.len() != 10
                || !valid_until.bytes().all(|b| b.is_ascii_digit() || b == b'-')
            {
                return Err(invalid("valid_until must be a YYYY-MM-DD date"));
            }
            args["valid_until"] = json!(valid_until);
        }
        self.call_tool("mnemosyne_remember", args).await
    }

    async fn recall(&self, input: MemoryInput) -> Result<Value, ToolError> {
        let query = input
            .query
            .as_deref()
            .ok_or_else(|| invalid("query is required"))?;
        if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS {
            return Err(invalid("query must be 1..=1000 characters"));
        }
        let mut args = json!({ "query": query });
        if let Some(limit) = input.limit {
            if !(1..=20).contains(&limit) {
                return Err(invalid("limit must be between 1 and 20"));
            }
            args["limit"] = json!(limit);
        }
        self.call_tool("mnemosyne_recall", args).await
    }

    async fn with_id(
        &self,
        input: MemoryInput,
        tool: &'static str,
    ) -> Result<Value, ToolError> {
        let id = Self::checked_id(&input)?;
        self.call_tool(tool, json!({ "memory_id": id })).await
    }

    async fn update(&self, input: MemoryInput) -> Result<Value, ToolError> {
        let id = Self::checked_id(&input)?;
        let content = Self::checked_content(&input)?;
        let mut args = json!({ "memory_id": id, "content": content });
        if let Some(importance) = input.importance {
            if !(0.0..=1.0).contains(&importance) {
                return Err(invalid("importance must be between 0.0 and 1.0"));
            }
            args["importance"] = json!(importance);
        }
        self.call_tool("mnemosyne_update", args).await
    }

    async fn call_tool(&self, tool: &str, args: Value) -> Result<Value, ToolError> {
        let session = self.open_session().await?;
        // The MCP handshake expects the initialized notification; failures
        // are tolerated because servers still accept tool calls afterwards.
        let _ = self
            .post_body(
                Some(&session),
                json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            )
            .await;
        // Futures are lazy: the tool call must be awaited to completion BEFORE
        // the session is closed, otherwise the DELETE races ahead of the
        // never-polled call and Mnemosyne answers "Session not found" (404).
        let call = self
            .post_body(
                Some(&session),
                json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": { "name": tool, "arguments": args }
                }),
            )
            .await;
        self.close_session(&session).await;
        Self::tool_result(&call?, tool)
    }

    async fn open_session(&self) -> Result<String, ToolError> {
        let response = self
            .post(
                None,
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {},
                        "clientInfo": {
                            "name": "infra-gateway",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                }),
            )
            .await?;
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        // Drain the initialize SSE body before the connection is reused or
        // dropped: aborting the stream mid-read makes the server tear down
        // the fresh session and every follow-up request returns 404.
        let _ = response.text().await;
        session.ok_or_else(|| {
            ToolError::new("mnemosyne_protocol", "Mnemosyne returned no session ID")
        })
    }

    async fn post(&self, session: Option<&str>, body: Value) -> Result<reqwest::Response, ToolError> {
        let mut request = self
            .client
            .post(self.base.as_str())
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.token),
            )
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .json(&body);
        if let Some(session) = session {
            request = request.header("mcp-session-id", session);
        }
        let response = request
            .send()
            .await
            .map_err(|_| ToolError::new("mnemosyne_unreachable", "Mnemosyne is unreachable"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                401 | 403 => ToolError::new("mnemosyne_auth", "Mnemosyne rejected the gateway token"),
                404 => ToolError::new("mnemosyne_protocol", "Mnemosyne MCP endpoint not found"),
                429 => ToolError::new("mnemosyne_busy", "Mnemosyne is rate-limiting the gateway"),
                _ => ToolError::new("mnemosyne_unreachable", "Mnemosyne request failed"),
            });
        }
        Ok(response)
    }

    async fn post_body(&self, session: Option<&str>, body: Value) -> Result<String, ToolError> {
        let response = self.post(session, body).await?;
        let text = response
            .text()
            .await
            .map_err(|_| ToolError::new("mnemosyne_protocol", "Mnemosyne response could not be read"))?;
        if text.len() > MAX_RESPONSE_BYTES {
            return Err(ToolError::new(
                "mnemosyne_protocol",
                "Mnemosyne response exceeded the size limit",
            ));
        }
        Ok(text)
    }

    async fn close_session(&self, session: &str) {
        let _ = self
            .client
            .delete(self.base.as_str())
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.token),
            )
            .header("mcp-session-id", session)
            .send()
            .await;
    }

    fn tool_result(text: &str, tool: &str) -> Result<Value, ToolError> {
        // Streamable HTTP replies may arrive SSE-framed; unwrap the data line.
        let payload = text
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap_or(text);
        let value: Value = serde_json::from_str(payload.trim()).map_err(|_| {
            ToolError::new("mnemosyne_protocol", "Mnemosyne returned an unparseable response")
        })?;
        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Mnemosyne error");
            return Err(ToolError::new("mnemosyne_error", message));
        }
        let Some(result) = value.get("result") else {
            return Err(ToolError::new("mnemosyne_protocol", "Mnemosyne returned no result"));
        };
        let mut parts: Vec<&str> = Vec::new();
        if let Some(items) = result.get("content").and_then(Value::as_array) {
            for item in items {
                if let Some(part) = item.get("text").and_then(Value::as_str) {
                    parts.push(part);
                }
            }
        }
        let joined = parts.join("\n");
        if result.get("isError").and_then(Value::as_bool).unwrap_or(false) {
            let message: String = if joined.is_empty() {
                "Mnemosyne tool call failed".into()
            } else {
                joined.chars().take(400).collect()
            };
            return Err(ToolError::new("mnemosyne_tool_error", message));
        }
        // Mnemosyne tools return JSON payloads as text; unwrap when possible.
        if let Ok(inner) = serde_json::from_str::<Value>(joined.trim()) {
            return Ok(inner);
        }
        Ok(json!({ "tool": tool, "text": joined }))
    }
}
