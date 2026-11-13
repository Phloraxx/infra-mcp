use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpExecInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Command to execute. Desktop Windows uses cmd.exe, Unix uses /bin/sh; Android is restricted. Use terminal for interactive state.
    pub(crate) command: String,
    #[serde(default)]
    /// Working directory for this operation only. Omit to use the agent default; it does not change later calls.
    pub(crate) cwd: Option<String>,
    #[serde(default)]
    /// Execution timeout in milliseconds: default 30000, clamped to 100..120000. Output is bounded; use jobs for long work.
    pub(crate) timeout_ms: Option<u64>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct McpExecOutput {
    pub(crate) request_id: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}
