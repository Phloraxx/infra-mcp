use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpContainerInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Choose an allowed action. Only that action is executed; provide its required fields.
    pub(crate) action: ContainerActionName,
    #[serde(default)]
    /// Required Docker container name/ID except for list. Docker must be available on the device.
    pub(crate) container: Option<String>,
    #[serde(default)]
    /// Maximum recent log lines; ask for a bounded relevant excerpt.
    pub(crate) tail: Option<usize>,
}
