//! Minimal MCP client for the harness. It speaks the same JSON-RPC over HTTP
//! shape the product uses, so a scenario exercises the real tool surface.

use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
pub struct McpClient {
    http: reqwest::Client,
    base: String,
    token: String,
    sequence: std::sync::Arc<AtomicU64>,
}

impl McpClient {
    pub fn new(base: &str, token: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: base.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
            sequence: std::sync::Arc::new(AtomicU64::new(1)),
        }
    }

    /// Call a tool and return its structured content. Tool-level failures keep
    /// their structured body (status, error_code, next_action) so a scenario can
    /// assert on the exact outcome instead of a transport error.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<Value, String> {
        let response = self
            .rpc("tools/call", json!({"name": tool, "arguments": arguments}))
            .await?;
        let result = response
            .get("result")
            .ok_or_else(|| format!("missing MCP result: {response}"))?;
        Ok(result
            .get("structuredContent")
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub async fn rpc(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let response = self
            .http
            .post(format!("{}/mcp", self.base))
            .bearer_auth(&self.token)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-06-18")
            .json(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .send()
            .await
            .map_err(|error| format!("MCP request failed: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("MCP body failed: {error}"))?;
        if !status.is_success() {
            return Err(format!("MCP HTTP {status}: {body}"));
        }
        serde_json::from_str(&body).map_err(|error| format!("MCP JSON failed: {error}: {body}"))
    }
}
