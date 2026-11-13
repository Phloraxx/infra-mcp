use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpProcessesInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Choose an allowed action. Only that action is executed; provide its required fields.
    pub(crate) action: ProcessActionName,
    #[serde(default)]
    /// Search/filter text. Content search uses regex unless literal is true.
    pub(crate) query: Option<String>,
    #[serde(default)]
    /// Required target process ID for terminate; inspect the process before terminating it.
    pub(crate) pid: Option<u32>,
    #[serde(default)]
    /// Request forceful termination; default false. OS permissions still apply.
    pub(crate) force: Option<bool>,
}
