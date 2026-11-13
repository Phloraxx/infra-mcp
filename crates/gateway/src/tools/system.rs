use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpSystemInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Choose an allowed action. Only that action is executed; provide its required fields.
    pub(crate) action: SystemActionName,
}
