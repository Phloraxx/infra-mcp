use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpTransferInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Choose an allowed action. Only that action is executed; provide its required fields.
    pub(crate) action: TransferActionName,
    #[serde(default)]
    /// Target path. Desktop paths follow the remote OS; Android paths must stay inside its authorized workspace.
    pub(crate) path: Option<String>,
    #[serde(default)]
    /// Required declared file size in bytes for upload_begin.
    pub(crate) size: Option<u64>,
    #[serde(default)]
    /// Optional expected SHA-256 digest; upload commit verifies it before final activation.
    pub(crate) sha256: Option<String>,
    #[serde(default)]
    /// Allow replacing an existing destination; default false.
    pub(crate) overwrite: Option<bool>,
    #[serde(default)]
    /// Create missing parent directories for the selected write/upload action; default false.
    pub(crate) create_parents: Option<bool>,
    #[serde(default)]
    /// Required returned transfer UUID for chunk/commit/status/cancel. Preserve it for resumption.
    pub(crate) transfer_id: Option<String>,
    #[serde(default)]
    /// Byte offset for range/output/chunk reads; preserve the returned next offset rather than counting characters.
    pub(crate) offset: Option<u64>,
    #[serde(default)]
    /// Required upload_chunk bytes encoded as base64; raw chunk cap is 256 KiB.
    pub(crate) data_base64: Option<String>,
    #[serde(default)]
    /// Maximum bytes for read/output/chunk actions, subject to agent caps. Does not limit directory list.
    pub(crate) length: Option<usize>,
}
