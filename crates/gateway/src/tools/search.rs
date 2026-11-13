use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpSearchInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Known remote directory to search. Bound the root to the relevant project.
    pub(crate) root: String,
    /// Search/filter text. Content search uses regex unless literal is true.
    pub(crate) query: String,
    /// files searches path names; content searches UTF-8 text. Binary and large files may be skipped.
    pub(crate) mode: SearchModeName,
    #[serde(default)]
    /// Treat search query as literal text instead of regex.
    pub(crate) literal: Option<bool>,
    #[serde(default)]
    /// Use case-insensitive matching; default false.
    pub(crate) ignore_case: Option<bool>,
    #[serde(default)]
    /// Include hidden paths; default false. Git-ignore rules still apply.
    pub(crate) include_hidden: Option<bool>,
    #[serde(default)]
    /// Optional glob to restrict matching file names or relative paths.
    pub(crate) file_pattern: Option<String>,
    #[serde(default)]
    /// Maximum search hits; default 100, cap 500. Inspect truncated and skipped-file counts.
    pub(crate) max_results: Option<usize>,
    #[serde(default)]
    /// Context lines around content matches; cap 5.
    pub(crate) context_lines: Option<usize>,
}
