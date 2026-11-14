use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpNetworkInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Choose an allowed action. Only that action is executed; provide its required fields.
    pub(crate) action: NetworkActionName,
    #[serde(default)]
    /// Required hostname/IP for resolve and tcp_check.
    pub(crate) host: Option<String>,
    #[serde(default)]
    /// Required TCP port for tcp_check.
    pub(crate) port: Option<u16>,
    #[serde(default)]
    /// TCP connection timeout in milliseconds: default 3000, clamped to 100..30000. Used only by tcp_check.
    pub(crate) timeout_ms: Option<u64>,
}
