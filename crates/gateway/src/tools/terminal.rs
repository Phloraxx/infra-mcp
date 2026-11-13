use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum McpTerminalActionInput {
    Open {
        /// Interactive command to start in a persistent PTY.
        command: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        rows: Option<u16>,
        #[serde(default)]
        cols: Option<u16>,
    },
    List,
    Read {
        /// PTY session UUID returned by open.
        session_id: String,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        length: Option<usize>,
    },
    Write {
        session_id: String,
        data: String,
    },
    Resize {
        session_id: String,
        rows: u16,
        cols: u16,
    },
    Close {
        session_id: String,
    },
}
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpTerminalInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    #[serde(flatten)]
    pub(crate) action: McpTerminalActionInput,
}
