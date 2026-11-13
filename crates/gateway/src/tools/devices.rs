use crate::*;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct McpDevicesInput {
    /// Optional. Set true for the small routing view (name, ID, online, platform, capabilities);
    /// omit or false to retain the complete legacy device metadata response.
    #[serde(default)]
    pub(crate) compact: bool,
}
