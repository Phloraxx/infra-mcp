use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpGitInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Choose an allowed action. Only that action is executed; provide its required fields.
    pub(crate) action: GitActionName,
    /// Required remote repository directory. Git credentials must already work noninteractively.
    pub(crate) repo: String,
    #[serde(default)]
    /// For diff, select staged changes; default false.
    pub(crate) staged: Option<bool>,
    #[serde(default)]
    /// Maximum Git log entries, subject to the agent cap.
    pub(crate) limit: Option<usize>,
    #[serde(default)]
    /// Optional Git remote name; defaults to the agent operation default.
    pub(crate) remote: Option<String>,
    #[serde(default)]
    /// Optional branch for pull/push. Pull is fast-forward-only.
    pub(crate) branch: Option<String>,
}
