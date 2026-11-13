use crate::*;

// Internally tagged action schemas let MCP clients see which parameters are
// actually required instead of a bag of optional fields.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum McpFileActionInput {
    Read {
        path: String,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        length: Option<usize>,
    },
    Write {
        path: String,
        content: String,
        #[serde(default)]
        create_parents: Option<bool>,
    },
    Append {
        path: String,
        content: String,
        #[serde(default)]
        create_parents: Option<bool>,
    },
    ExactEdit {
        path: String,
        old: String,
        new: String,
        #[serde(default)]
        expected_replacements: Option<usize>,
    },
    Hash {
        path: String,
    },
    List {
        path: String,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        length: Option<usize>,
    },
    Stat {
        path: String,
    },
    Mkdir {
        path: String,
    },
    Copy {
        path: String,
        destination: String,
        #[serde(default)]
        recursive: Option<bool>,
    },
    Move {
        path: String,
        destination: String,
    },
    Trash {
        path: String,
    },
    DeletePermanent {
        path: String,
        #[serde(default)]
        recursive: Option<bool>,
    },
}
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpFilesInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    #[serde(flatten)]
    pub(crate) action: McpFileActionInput,
}
