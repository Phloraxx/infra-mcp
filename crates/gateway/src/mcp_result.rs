//! Keep existing JSON fields while exposing actionable MCP tool failures.
use rmcp::{
    handler::server::tool::IntoCallToolResult,
    model::{CallToolResponse, CallToolResult, ContentBlock},
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};
use std::borrow::Cow;

pub(crate) struct ToolJson<T>(pub T);
impl<T: JsonSchema> JsonSchema for ToolJson<T> {
    fn schema_name() -> Cow<'static, str> {
        T::schema_name()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        T::json_schema(generator)
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub next_action: &'static str,
}
impl ToolError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        let (retryable, next_action) = match code {
            "invalid_argument" => (false, "Correct the named argument using the tool schema."),
            "device_not_found" | "ambiguous_device" => (
                false,
                "Call devices and select a returned unique device ID.",
            ),
            "device_offline" | "device_connection_closed" => (
                false,
                "Check device connectivity; do not reinstall or replay uncertain work.",
            ),
            "device_revoked" => (
                false,
                "Owner action is required to deliberately pair this device again.",
            ),
            "unsupported" => (
                false,
                "Choose an action advertised by this device; platform restrictions still apply.",
            ),
            "unauthorized" | "permission_required" => (
                false,
                "Check the required authorization or local OS permission.",
            ),
            "agent_busy" => (
                true,
                "The operation was rejected before execution; retry after a short backoff.",
            ),
            "outcome_unknown" => (
                false,
                "Inspect resulting state or existing job/transfer handles before any retry. Never replay automatically.",
            ),
            "deadline_exceeded" => (
                false,
                "Inspect partial results and side effects; use jobs for intentional long work.",
            ),
            _ => (
                false,
                "Inspect the returned error, stdout/stderr or existing state before retrying.",
            ),
        };
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
            next_action,
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_argument", message)
    }
}
impl From<String> for ToolError {
    fn from(message: String) -> Self {
        let prefix = message.split(':').next().unwrap_or("");
        let code = match prefix {
            "device_not_found"
            | "ambiguous_device"
            | "device_offline"
            | "device_revoked"
            | "device_registry_error"
            | "device_connection_closed"
            | "agent_busy" => prefix,
            _ => "operation_failed",
        };
        Self::new(code, message.clone())
    }
}
impl From<&str> for ToolError {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}
impl IntoCallToolResult for ToolError {
    fn into_call_tool_result(self) -> Result<CallToolResponse, rmcp::ErrorData> {
        Ok(CallToolResult::structured_error(json!({
            "status":"failed", "error":self.message, "error_code":self.code,
            "retryable":self.retryable, "next_action":self.next_action,
        }))
        .into())
    }
}
impl<T: Serialize + JsonSchema + 'static> IntoCallToolResult for ToolJson<T> {
    fn into_call_tool_result(self) -> Result<CallToolResponse, rmcp::ErrorData> {
        let mut value = serde_json::to_value(self.0)
            .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))?;
        let status = value.get("status").and_then(Value::as_str).unwrap_or("");
        let code = match status {
            "failed" | "canceled" | "deadline_exceeded" | "outcome_unknown" | "unsupported"
            | "unauthorized" => Some(status),
            "ok" if value
                .get("exit_code")
                .and_then(Value::as_i64)
                .is_some_and(|code| code != 0) =>
            {
                Some("command_failed")
            }
            _ => None,
        };
        if let Some(code) = code {
            let message = value.get("error").and_then(Value::as_str).unwrap_or(code);
            // Only the agent's explicit pre-execution overload rejection is retryable.
            let code = if code == "failed" && message.starts_with("agent_busy:") {
                "agent_busy"
            } else {
                code
            };
            let error = ToolError::new(code, message);
            if let Some(object) = value.as_object_mut() {
                object.insert("error_code".into(), json!(error.code));
                object.insert("retryable".into(), json!(error.retryable));
                object.insert("next_action".into(), json!(error.next_action));
            }
            Ok(CallToolResult::structured_error(value).into())
        } else {
            // Native image blocks let multimodal clients see a screenshot rather
            // than spending model context on a base64 string. Keep structured
            // bytes for existing JSON consumers; text contains only metadata.
            let image = value.get("body").and_then(|body| {
                let mime = body.get("mime_type")?.as_str()?;
                let data = body.get("data_base64")?.as_str()?;
                matches!(mime, "image/jpeg" | "image/png")
                    .then(|| (mime.to_owned(), data.to_owned()))
            });
            let mut result = CallToolResult::structured(value.clone());
            if let Some((mime, data)) = image {
                if let Some(body) = value.get_mut("body").and_then(Value::as_object_mut) {
                    body.remove("data_base64");
                }
                result.content = vec![
                    ContentBlock::text(value.to_string()),
                    ContentBlock::image(data, mime),
                ];
            }
            Ok(result.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn complete(result: CallToolResponse) -> CallToolResult {
        let CallToolResponse::Complete(result) = result else {
            panic!("expected complete result")
        };
        result
    }
    #[test]
    fn unsupported_and_nonzero_exit_are_mcp_errors_with_original_fields() {
        for value in [
            json!({"status":"unsupported","error":"network"}),
            json!({"status":"ok","exit_code":7,"stdout":"partial","stderr":"bad input"}),
        ] {
            let result = complete(ToolJson(value.clone()).into_call_tool_result().unwrap());
            assert_eq!(result.is_error, Some(true));
            let body = result.structured_content.unwrap();
            assert_eq!(body["status"], value["status"]);
            assert_eq!(body["retryable"], false);
            assert!(body["error_code"].is_string());
        }
    }
    #[test]
    fn uncertain_results_never_recommend_automatic_retry() {
        let result = complete(ToolJson(json!({"status":"outcome_unknown","request_id":"fixture-request","error":"response lost"})).into_call_tool_result().unwrap());
        let body = result.structured_content.unwrap();
        assert_eq!(body["request_id"], "fixture-request");
        assert_eq!(body["retryable"], false);
        assert!(
            body["next_action"]
                .as_str()
                .unwrap()
                .contains("Never replay")
        );
    }
    #[test]
    fn offline_and_invalid_arguments_have_distinct_error_codes() {
        let offline = complete(
            ToolError::from("device_offline: fixture is disconnected")
                .into_call_tool_result()
                .unwrap(),
        );
        let invalid = complete(
            ToolError::invalid("port is required")
                .into_call_tool_result()
                .unwrap(),
        );
        assert_eq!(
            offline.structured_content.unwrap()["error_code"],
            "device_offline"
        );
        assert_eq!(
            invalid.structured_content.unwrap()["error_code"],
            "invalid_argument"
        );
    }
    #[test]
    fn partial_transfer_and_device_arrays_are_successful_results() {
        for value in [
            json!({"status":"partial","bytes_transferred":42}),
            json!([]),
        ] {
            let result = complete(ToolJson(value.clone()).into_call_tool_result().unwrap());
            assert_eq!(result.is_error, Some(false));
            assert_eq!(result.structured_content, Some(value));
        }
    }

    #[test]
    fn screenshot_has_native_image_content_without_base64_in_text() {
        let result = complete(ToolJson(json!({
            "status":"ok", "body":{"mime_type":"image/png","size":5,"data_base64":"Zml4dHVyZQ=="}
        })).into_call_tool_result().unwrap());
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["content"][1]["type"], "image");
        assert_eq!(wire["content"][1]["mimeType"], "image/png");
        assert!(
            !wire["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Zml4dHVyZQ==")
        );
        assert_eq!(
            wire["structuredContent"]["body"]["data_base64"],
            "Zml4dHVyZQ=="
        );
    }
}
