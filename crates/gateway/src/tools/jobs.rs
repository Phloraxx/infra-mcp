use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum McpJobActionInput {
    Start {
        /// Noninteractive command (Windows cmd.exe; Unix /bin/sh). Use terminal for interactive work.
        command: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    List {
        /// Optional. Short list with job ID/status/exit_code and timestamps, excluding the command.
        #[serde(default)]
        compact: Option<bool>,
    },
    Read {
        /// UUID returned by start; reuse it across reconnects.
        job_id: String,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        length: Option<usize>,
    },
    Wait {
        /// UUID returned by start; do not start another job to check progress.
        job_id: String,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        length: Option<usize>,
        /// Bounded long-poll milliseconds, default 10000, max 20000.
        #[serde(default)]
        wait_timeout_ms: Option<u64>,
    },
    Cancel {
        job_id: String,
    },
}
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpJobsInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    #[serde(flatten)]
    pub(crate) action: McpJobActionInput,
}
