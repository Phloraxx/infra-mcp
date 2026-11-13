use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpCopyInput {
    /// Source device ID or unique name from devices; transfer capability is required.
    pub(crate) source_device: String,
    /// Destination device ID or unique name; transfer capability is required.
    pub(crate) destination_device: String,
    /// Source file path on source_device.
    pub(crate) source_path: String,
    /// Destination file path on destination_device.
    pub(crate) destination_path: String,
    #[serde(default)]
    /// Allow replacing an existing destination; default false.
    pub(crate) overwrite: Option<bool>,
    #[serde(default)]
    /// Create missing parent directories for the selected write/upload action; default false.
    pub(crate) create_parents: Option<bool>,
    #[serde(default)]
    /// Source download UUID from a previous partial copy; resume requires both transfer IDs.
    pub(crate) source_transfer_id: Option<String>,
    #[serde(default)]
    /// Destination upload UUID from a previous partial copy; inspect status before retrying uncertain chunks.
    pub(crate) destination_transfer_id: Option<String>,
    /// Max bytes relayed this request (default 16 MiB, cap 64 MiB).
    #[serde(default)]
    pub(crate) max_bytes: Option<u64>,
}
