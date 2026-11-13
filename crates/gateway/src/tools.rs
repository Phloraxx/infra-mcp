use super::*;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct McpDevicesInput {
    /// Optional. Set true for the small routing view (name, ID, online, platform, capabilities);
    /// omit or false to retain the complete legacy device metadata response.
    #[serde(default)]
    pub(crate) compact: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpExecInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Command to execute. Desktop Windows uses cmd.exe, Unix uses /bin/sh; Android is restricted. Use terminal for interactive state.
    pub(crate) command: String,
    #[serde(default)]
    /// Working directory for this operation only. Omit to use the agent default; it does not change later calls.
    pub(crate) cwd: Option<String>,
    #[serde(default)]
    /// Execution timeout in milliseconds: default 30000, clamped to 100..120000. Output is bounded; use jobs for long work.
    pub(crate) timeout_ms: Option<u64>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct McpExecOutput {
    pub(crate) request_id: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

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
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpProcessesInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Choose an allowed action. Only that action is executed; provide its required fields.
    pub(crate) action: ProcessActionName,
    #[serde(default)]
    /// Search/filter text. Content search uses regex unless literal is true.
    pub(crate) query: Option<String>,
    #[serde(default)]
    /// Required target process ID for terminate; inspect the process before terminating it.
    pub(crate) pid: Option<u32>,
    #[serde(default)]
    /// Request forceful termination; default false. OS permissions still apply.
    pub(crate) force: Option<bool>,
}
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

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpDesktopInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Choose an allowed action. Only that action is executed; provide its required fields.
    pub(crate) action: DesktopActionName,
    #[serde(default)]
    /// Optional display identifier returned by displays.
    pub(crate) display: Option<u32>,
    #[serde(default)]
    /// Pixel x coordinate; required for click. For a screenshot region supply x, y, width and height together.
    pub(crate) x: Option<i32>,
    #[serde(default)]
    /// Pixel y coordinate; required for click. Screenshot regions require all four bounds.
    pub(crate) y: Option<i32>,
    #[serde(default)]
    /// Screenshot-region width in pixels; requires x, y and height.
    pub(crate) width: Option<u32>,
    #[serde(default)]
    /// Screenshot-region height in pixels; requires x, y and width.
    pub(crate) height: Option<u32>,
    #[serde(default)]
    /// Required application identifier/name for focus. Local desktop permission may be needed.
    pub(crate) app: Option<String>,
    #[serde(default)]
    /// Required text for type_text/clipboard_set. These actions change the device UI or clipboard.
    pub(crate) text: Option<String>,
    #[serde(default)]
    /// Required key name for key; combine with modifiers when needed.
    pub(crate) key: Option<String>,
    #[serde(default)]
    /// Optional modifier names for key; default empty.
    pub(crate) modifiers: Option<Vec<String>>,
    #[serde(default)]
    /// Optional click button; defaults to primary. Availability depends on the platform.
    pub(crate) button: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpSystemInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Choose an allowed action. Only that action is executed; provide its required fields.
    pub(crate) action: SystemActionName,
}

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

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpServiceInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Choose an allowed action. Only that action is executed; provide its required fields.
    pub(crate) action: ServiceActionName,
    #[serde(default)]
    /// Required service name for status/start/stop/restart. Linux services are user scope.
    pub(crate) name: Option<String>,
    #[serde(default)]
    /// Search/filter text. Content search uses regex unless literal is true.
    pub(crate) query: Option<String>,
}

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

#[derive(Clone)]
pub(crate) struct InfraMcp {
    pub(crate) state: AppState,
}
impl InfraMcp {
    pub(crate) fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[tool_router(server_handler)]
impl InfraMcp {
    #[tool(
        name = "devices",
        description = "Discover connected devices. Use compact=true for minimal name/ID/online/platform/capabilities routing metadata, otherwise get all fields. Use the returned unique device ID if names are ambiguous; Android capabilities are restricted."
    )]
    async fn devices(
        &self,
        Parameters(input): Parameters<McpDevicesInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let devices = device_views(&self.state)
            .await
            .map_err(|error| ToolError::new("device_registry_error", error.to_string()))?;
        if input.compact {
            return Ok(McpJson(serde_json::json!({
                "devices": devices.iter().map(|device| serde_json::json!({
                    "device_id": device.device_id,
                    "device_name": device.device_name,
                    "platform": device.platform,
                    "online": device.online,
                    "capabilities": device.capabilities,
                })).collect::<Vec<_>>()
            })));
        }
        Ok(McpJson(serde_json::json!({ "devices": devices })))
    }

    #[tool(
        name = "exec",
        description = "Run a short noninteractive command as the device user. Windows uses cmd.exe; Unix uses /bin/sh; Android exec is restricted. Default 30s, maximum 120s. Use jobs for long work and terminal for interactive/stateful work. Inspect exit_code; never replay outcome_unknown."
    )]
    async fn exec(
        &self,
        Parameters(input): Parameters<McpExecInput>,
    ) -> Result<McpJson<McpExecOutput>, ToolError> {
        let device_id = resolve_device(&self.state, &input.device)
            .await
            .map_err(dispatch_error)?;
        let response = dispatch_to_device(
            &self.state,
            device_id.clone(),
            AgentOperation::Exec {
                command: input.command,
                cwd: input.cwd,
                timeout_ms: input.timeout_ms,
            },
            input.timeout_ms.unwrap_or(30_000).clamp(100, 120_000) + 5_000,
        )
        .await
        .map_err(dispatch_error)?;
        record_operation(&self.state, &device_id, "exec", &response);
        Ok(McpJson(exec_output(response)))
    }

    #[tool(
        name = "files",
        description = "Read or mutate files on an Infra device. Actions: read, write, append, exact_edit, hash, list, stat, mkdir, copy, move, trash, delete_permanent."
    )]
    async fn files(
        &self,
        Parameters(input): Parameters<McpFilesInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let action = match input.action {
            McpFileActionInput::Read {
                path,
                offset,
                length,
            } => FileAction::Read {
                path,
                offset,
                length,
            },
            McpFileActionInput::Write {
                path,
                content,
                create_parents,
            } => FileAction::Write {
                path,
                content,
                create_parents: create_parents.unwrap_or(false),
            },
            McpFileActionInput::Append {
                path,
                content,
                create_parents,
            } => FileAction::Append {
                path,
                content,
                create_parents: create_parents.unwrap_or(false),
            },
            McpFileActionInput::ExactEdit {
                path,
                old,
                new,
                expected_replacements,
            } => FileAction::ExactEdit {
                path,
                old,
                new,
                expected_replacements,
            },
            McpFileActionInput::Hash { path } => FileAction::Hash { path },
            McpFileActionInput::List {
                path,
                offset,
                length,
            } => {
                if length.is_some_and(|n| n == 0 || n > MAX_DIRECTORY_PAGE_SIZE) {
                    return Err(ToolError::invalid(format!(
                        "files.list length must be between 1 and {MAX_DIRECTORY_PAGE_SIZE} entries"
                    )));
                }
                if offset.is_some_and(|n| n > MAX_DIRECTORY_ENTRIES as u64) {
                    return Err(ToolError::invalid(format!(
                        "files.list offset must not exceed {MAX_DIRECTORY_ENTRIES} entries"
                    )));
                }
                FileAction::List {
                    path,
                    offset,
                    length,
                }
            }
            McpFileActionInput::Stat { path } => FileAction::Stat { path },
            McpFileActionInput::Mkdir { path } => FileAction::Mkdir { path },
            McpFileActionInput::Copy {
                path,
                destination,
                recursive,
            } => FileAction::Copy {
                source: path,
                destination,
                recursive: recursive.unwrap_or(false),
            },
            McpFileActionInput::Move { path, destination } => FileAction::Move {
                source: path,
                destination,
            },
            McpFileActionInput::Trash { path } => FileAction::Trash { path },
            McpFileActionInput::DeletePermanent { path, recursive } => {
                FileAction::DeletePermanent {
                    path,
                    recursive: recursive.unwrap_or(false),
                }
            }
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Files(FileRequest { action }),
            65_000,
        )
        .await
    }

    #[tool(
        name = "search",
        description = "Search filenames or text content efficiently on an Infra device. mode: files or content."
    )]
    async fn search(
        &self,
        Parameters(input): Parameters<McpSearchInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let mode = match input.mode.as_str() {
            "files" => SearchMode::Files,
            "content" => SearchMode::Content,
            _ => return Err("search mode must be files or content".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Search(SearchRequest {
                root: input.root,
                query: input.query,
                mode,
                literal: input.literal.unwrap_or(false),
                ignore_case: input.ignore_case.unwrap_or(true),
                include_hidden: input.include_hidden.unwrap_or(false),
                file_pattern: input.file_pattern,
                max_results: input.max_results,
                context_lines: input.context_lines,
            }),
            65_000,
        )
        .await
    }

    #[tool(
        name = "processes",
        description = "List/find or terminate processes on an Infra device. Actions: list, terminate."
    )]
    async fn processes(
        &self,
        Parameters(input): Parameters<McpProcessesInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let action = match input.action.as_str() {
            "list" => ProcessAction::List { query: input.query },
            "terminate" => ProcessAction::Terminate {
                pid: input.pid.ok_or("pid is required")?,
                force: input.force.unwrap_or(false),
            },
            _ => return Err("unknown processes action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Processes(ProcessRequest { action }),
            30_000,
        )
        .await
    }

    #[tool(
        name = "jobs",
        description = "Manage durable background work. start: command; list: optional compact=true; read/wait/cancel: job_id; read/wait: offset and length. wait blocks for new output or completion (default 10s, max 20s). Reuse IDs and next_offset; do not replay uncertain work."
    )]
    async fn jobs(
        &self,
        Parameters(input): Parameters<McpJobsInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let compact_list = matches!(
            &input.action,
            McpJobActionInput::List {
                compact: Some(true)
            }
        );
        if matches!(&input.action, McpJobActionInput::Wait { .. }) {
            // Older enrolled agents may understand "jobs" but not the newer
            // wait variant. Avoid sending a wire enum they cannot deserialize.
            let device_id = resolve_device(&self.state, &input.device)
                .await
                .map_err(dispatch_error)?;
            if let Some(connection) = self.state.devices.read().await.get(&device_id)
                && !connection
                    .hello
                    .capabilities
                    .iter()
                    .any(|c| c == "jobs.wait_v1")
            {
                return Err(ToolError::new(
                    "unsupported",
                    "jobs.wait requires an agent advertising jobs.wait_v1; use jobs.read until it is upgraded",
                ));
            }
        }
        let action = match input.action {
            McpJobActionInput::Start {
                command,
                cwd,
                timeout_ms,
            } => JobAction::Start {
                command,
                cwd,
                timeout_ms,
            },
            McpJobActionInput::List { .. } => JobAction::List,
            McpJobActionInput::Read {
                job_id,
                offset,
                length,
            } => JobAction::Read {
                job_id: parse_uuid(Some(job_id), "job_id")?,
                offset,
                length,
            },
            McpJobActionInput::Wait {
                job_id,
                offset,
                length,
                wait_timeout_ms,
            } => JobAction::Wait {
                job_id: parse_uuid(Some(job_id), "job_id")?,
                offset,
                length,
                wait_timeout_ms,
            },
            McpJobActionInput::Cancel { job_id } => JobAction::Cancel {
                job_id: parse_uuid(Some(job_id), "job_id")?,
            },
        };
        let mut response = tool_operation(
            &self.state,
            input.device,
            AgentOperation::Jobs(JobRequest { action }),
            65_000,
        )
        .await?;
        if compact_list
            && let Some(jobs) = response
                .0
                .pointer_mut("/body/jobs")
                .and_then(|v| v.as_array_mut())
        {
            for job in jobs {
                *job = serde_json::json!({
                    "id":job["id"],
                    "status":job["status"],
                    "exit_code":job["exit_code"],
                    "started_at_unix_ms":job["started_at_unix_ms"],
                    "finished_at_unix_ms":job["finished_at_unix_ms"],
                    "output_truncated":job["output_truncated"]
                });
            }
        }
        Ok(response)
    }

    #[tool(
        name = "terminal",
        description = "Manage persistent PTY sessions. open requires command; read/write/resize/close require session_id; write also requires data, resize requires rows and cols. Reads use byte offsets and can report expired retention. Reattach using the same ID; never replay uncertain input."
    )]
    async fn terminal(
        &self,
        Parameters(input): Parameters<McpTerminalInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let action = match input.action {
            McpTerminalActionInput::Open {
                command,
                cwd,
                rows,
                cols,
            } => TerminalAction::Open {
                command,
                cwd,
                rows,
                cols,
            },
            McpTerminalActionInput::List => TerminalAction::List,
            McpTerminalActionInput::Read {
                session_id,
                offset,
                length,
            } => TerminalAction::Read {
                session_id: parse_uuid(Some(session_id), "session_id")?,
                offset,
                length,
            },
            McpTerminalActionInput::Write { session_id, data } => TerminalAction::Write {
                session_id: parse_uuid(Some(session_id), "session_id")?,
                data,
            },
            McpTerminalActionInput::Resize {
                session_id,
                rows,
                cols,
            } => TerminalAction::Resize {
                session_id: parse_uuid(Some(session_id), "session_id")?,
                rows,
                cols,
            },
            McpTerminalActionInput::Close { session_id } => TerminalAction::Close {
                session_id: parse_uuid(Some(session_id), "session_id")?,
            },
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Terminal(TerminalRequest { action }),
            65_000,
        )
        .await
    }

    #[tool(
        name = "transfer",
        description = "Resumable binary upload/download on an Infra device. Actions: upload_begin, upload_chunk, upload_commit, download_begin, download_chunk, status, list, cancel."
    )]
    async fn transfer(
        &self,
        Parameters(input): Parameters<McpTransferInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let action = match input.action.as_str() {
            "upload_begin" => TransferAction::UploadBegin {
                path: required(input.path, "path")?,
                size: required(input.size, "size")?,
                sha256: input.sha256,
                overwrite: input.overwrite.unwrap_or(false),
                create_parents: input.create_parents.unwrap_or(false),
            },
            "upload_chunk" => TransferAction::UploadChunk {
                transfer_id: parse_uuid(input.transfer_id, "transfer_id")?,
                offset: input.offset.ok_or("offset is required")?,
                data_base64: required(input.data_base64, "data_base64")?,
            },
            "upload_commit" => TransferAction::UploadCommit {
                transfer_id: parse_uuid(input.transfer_id, "transfer_id")?,
            },
            "download_begin" => TransferAction::DownloadBegin {
                path: required(input.path, "path")?,
            },
            "download_chunk" => TransferAction::DownloadChunk {
                transfer_id: parse_uuid(input.transfer_id, "transfer_id")?,
                offset: input.offset.ok_or("offset is required")?,
                length: input.length,
            },
            "status" => TransferAction::Status {
                transfer_id: parse_uuid(input.transfer_id, "transfer_id")?,
            },
            "list" => TransferAction::List,
            "cancel" => TransferAction::Cancel {
                transfer_id: parse_uuid(input.transfer_id, "transfer_id")?,
            },
            _ => return Err("unknown transfer action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Transfer(TransferRequest { action }),
            300_000,
        )
        .await
    }

    #[tool(
        name = "hermes",
        description = "Delegate a task to the configured Hermes profile (start), inspect/cancel runs (status/stop), or list/read its API-owned sessions (sessions/messages). Start requires a stable request_id for idempotency. Capabilities checks are cheap; actual tasks may invoke models and Infra tools."
    )]
    async fn hermes(
        &self,
        Parameters(input): Parameters<HermesInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let bridge = self.state.hermes.as_ref().ok_or_else(|| {
            ToolError::new("unsupported", "Hermes bridge is disabled or misconfigured")
        })?;
        Ok(McpJson(bridge.run(input).await?))
    }

    #[tool(
        name = "hermes_memory",
        description = "Read or change the configured Hermes profile's built-in memory (MEMORY.md / USER.md). Uses Hermes MemoryStore for locked validated writes. Actions: read, add, patch, remove. Writes are disabled unless separately enabled by the operator. Read before changing an entry."
    )]
    async fn hermes_memory(
        &self,
        Parameters(input): Parameters<HermesMemoryInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let bridge = self.state.hermes.as_ref().ok_or_else(|| {
            ToolError::new("unsupported", "Hermes bridge is disabled or misconfigured")
        })?;
        Ok(McpJson(bridge.memory(input).await?))
    }

    #[tool(
        name = "copy",
        description = "Copy a file between enrolled Infra devices by name or ID. SHA-256 verified, resumable, bounded to 16 MiB per call by default. For partial results call again with both transfer IDs. Never auto-retry an uncertain chunk."
    )]
    async fn copy(
        &self,
        Parameters(input): Parameters<McpCopyInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        Ok(McpJson(copy::run(&self.state, input).await?))
    }

    #[tool(
        name = "desktop",
        description = "Desktop control on an Infra device. Actions: permissions, displays, windows, screenshot, focus, click, type_text, key, clipboard_get, clipboard_set."
    )]
    async fn desktop(
        &self,
        Parameters(input): Parameters<McpDesktopInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let action = match input.action.as_str() {
            "permissions" => DesktopAction::Permissions,
            "displays" => DesktopAction::Displays,
            "windows" => DesktopAction::Windows,
            "screenshot" => DesktopAction::Screenshot {
                display: input.display,
                x: input.x,
                y: input.y,
                width: input.width,
                height: input.height,
            },
            "focus" => DesktopAction::Focus {
                app: required(input.app, "app")?,
            },
            "click" => DesktopAction::Click {
                x: input.x.ok_or("x is required")?,
                y: input.y.ok_or("y is required")?,
                button: input.button,
            },
            "type_text" => DesktopAction::TypeText {
                text: required(input.text, "text")?,
            },
            "key" => DesktopAction::Key {
                key: required(input.key, "key")?,
                modifiers: input.modifiers.unwrap_or_default(),
            },
            "clipboard_get" => DesktopAction::ClipboardGet,
            "clipboard_set" => DesktopAction::ClipboardSet {
                text: required(input.text, "text")?,
            },
            _ => return Err("unknown desktop action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Desktop(DesktopRequest { action }),
            65_000,
        )
        .await
    }

    #[tool(
        name = "system",
        description = "Structured system information. Actions: summary, disks, networks."
    )]
    async fn system(
        &self,
        Parameters(input): Parameters<McpSystemInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let action = match input.action.as_str() {
            "summary" => SystemAction::Summary,
            "disks" => SystemAction::Disks,
            "networks" => SystemAction::Networks,
            _ => return Err("unknown system action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::System(SystemRequest { action }),
            30_000,
        )
        .await
    }

    #[tool(
        name = "network",
        description = "Structured network diagnostics. Actions: listeners, resolve, tcp_check."
    )]
    async fn network(
        &self,
        Parameters(input): Parameters<McpNetworkInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let action = match input.action.as_str() {
            "listeners" => NetworkAction::Listeners,
            "resolve" => NetworkAction::Resolve {
                host: required(input.host, "host")?,
            },
            "tcp_check" => NetworkAction::TcpCheck {
                host: required(input.host, "host")?,
                port: required(input.port, "port")?,
                timeout_ms: input.timeout_ms,
            },
            _ => return Err("unknown network action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Network(NetworkRequest { action }),
            45_000,
        )
        .await
    }

    #[tool(
        name = "services",
        description = "User-level/native service control without shell interpolation. Actions: list, status, start, stop, restart."
    )]
    async fn services(
        &self,
        Parameters(input): Parameters<McpServiceInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let action = match input.action.as_str() {
            "list" => ServiceAction::List { query: input.query },
            "status" => ServiceAction::Status {
                name: required(input.name, "name")?,
            },
            "start" => ServiceAction::Start {
                name: required(input.name, "name")?,
            },
            "stop" => ServiceAction::Stop {
                name: required(input.name, "name")?,
            },
            "restart" => ServiceAction::Restart {
                name: required(input.name, "name")?,
            },
            _ => return Err("unknown services action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Services(ServiceRequest { action }),
            65_000,
        )
        .await
    }

    #[tool(
        name = "containers",
        description = "Docker container operations using direct argv execution. Actions: list, inspect, logs, start, stop, restart."
    )]
    async fn containers(
        &self,
        Parameters(input): Parameters<McpContainerInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let action = match input.action.as_str() {
            "list" => ContainerAction::List,
            "inspect" => ContainerAction::Inspect {
                container: required(input.container, "container")?,
            },
            "logs" => ContainerAction::Logs {
                container: required(input.container, "container")?,
                tail: input.tail,
            },
            "start" => ContainerAction::Start {
                container: required(input.container, "container")?,
            },
            "stop" => ContainerAction::Stop {
                container: required(input.container, "container")?,
            },
            "restart" => ContainerAction::Restart {
                container: required(input.container, "container")?,
            },
            _ => return Err("unknown containers action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Containers(ContainerRequest { action }),
            90_000,
        )
        .await
    }

    #[tool(
        name = "git",
        description = "Structured Git operations without a shell. Actions: status, diff, log, branches, remotes, fetch, pull, push."
    )]
    async fn git(
        &self,
        Parameters(input): Parameters<McpGitInput>,
    ) -> Result<McpJson<serde_json::Value>, ToolError> {
        let action = match input.action.as_str() {
            "status" => GitAction::Status { repo: input.repo },
            "diff" => GitAction::Diff {
                repo: input.repo,
                staged: input.staged.unwrap_or(false),
            },
            "log" => GitAction::Log {
                repo: input.repo,
                limit: input.limit,
            },
            "branches" => GitAction::Branches { repo: input.repo },
            "remotes" => GitAction::Remotes { repo: input.repo },
            "fetch" => GitAction::Fetch {
                repo: input.repo,
                remote: input.remote,
            },
            "pull" => GitAction::Pull {
                repo: input.repo,
                remote: input.remote,
                branch: input.branch,
            },
            "push" => GitAction::Push {
                repo: input.repo,
                remote: input.remote,
                branch: input.branch,
            },
            _ => return Err("unknown git action".into()),
        };
        tool_operation(
            &self.state,
            input.device,
            AgentOperation::Git(GitRequest { action }),
            130_000,
        )
        .await
    }
}

pub(crate) async fn tool_operation(
    state: &AppState,
    device: String,
    operation: AgentOperation,
    timeout_ms: u64,
) -> Result<McpJson<serde_json::Value>, ToolError> {
    let action = operation_label(&operation);
    let device = resolve_device(state, &device)
        .await
        .map_err(dispatch_error)?;
    let response = dispatch_to_device(state, device.clone(), operation, timeout_ms)
        .await
        .map_err(dispatch_error)?;
    record_operation(state, &device, action, &response);
    Ok(McpJson(response_json(response)))
}

pub(crate) fn operation_label(operation: &AgentOperation) -> &'static str {
    match operation {
        AgentOperation::Status => "status",
        AgentOperation::Exec { .. } => "exec",
        AgentOperation::Files(_) => "files",
        AgentOperation::Search(_) => "search",
        AgentOperation::Processes(_) => "processes",
        AgentOperation::Jobs(_) => "jobs",
        AgentOperation::Terminal(_) => "terminal",
        AgentOperation::Transfer(_) => "transfer",
        AgentOperation::System(_) => "system",
        AgentOperation::Network(_) => "network",
        AgentOperation::Services(_) => "services",
        AgentOperation::Containers(_) => "containers",
        AgentOperation::Git(_) => "git",
        AgentOperation::Desktop(_) => "desktop",
    }
}

pub(crate) fn outcome_label(result: &OperationResult) -> &'static str {
    match result {
        OperationResult::Ok { .. } => "ok",
        OperationResult::Failed { .. } => "failed",
        OperationResult::Canceled { .. } => "canceled",
        OperationResult::DeadlineExceeded => "deadline_exceeded",
        OperationResult::OutcomeUnknown { .. } => "outcome_unknown",
        OperationResult::Unsupported { .. } => "unsupported",
        OperationResult::Unauthorized => "unauthorized",
    }
}

pub(crate) fn record_operation(
    state: &AppState,
    device_id: &str,
    action: &str,
    response: &AgentResponse,
) {
    let _ = state.store.record_activity(
        Some(device_id),
        action,
        outcome_label(&response.result),
        Some(&response.id.to_string()),
    );
    state.emit("activity", Some(device_id.to_string()));
}
pub(crate) fn response_json(response: AgentResponse) -> serde_json::Value {
    match response.result {
        OperationResult::Ok { body } => {
            serde_json::json!({"request_id":response.id,"status":"ok","body":body})
        }
        OperationResult::Failed { error } => {
            serde_json::json!({"request_id":response.id,"status":"failed","error":error})
        }
        OperationResult::Canceled { error } => {
            serde_json::json!({"request_id":response.id,"status":"canceled","error":error})
        }
        OperationResult::DeadlineExceeded => {
            serde_json::json!({"request_id":response.id,"status":"deadline_exceeded"})
        }
        OperationResult::OutcomeUnknown { error } => {
            serde_json::json!({"request_id":response.id,"status":"outcome_unknown","error":error})
        }
        OperationResult::Unsupported { operation } => {
            serde_json::json!({"request_id":response.id,"status":"unsupported","error":operation})
        }
        OperationResult::Unauthorized => {
            serde_json::json!({"request_id":response.id,"status":"unauthorized"})
        }
    }
}
pub(crate) fn exec_output(response: AgentResponse) -> McpExecOutput {
    let mut out = McpExecOutput {
        request_id: response.id.to_string(),
        status: "unknown".into(),
        exit_code: None,
        stdout: None,
        stderr: None,
        truncated: None,
        error: None,
    };
    match response.result {
        OperationResult::Ok { body } => {
            out.status = "ok".into();
            match serde_json::from_value::<ExecOutput>(body) {
                Ok(v) => {
                    out.exit_code = v.exit_code;
                    out.stdout = Some(v.stdout);
                    out.stderr = Some(v.stderr);
                    out.truncated = Some(v.truncated)
                }
                Err(e) => {
                    out.status = "failed".into();
                    out.error = Some(format!("invalid exec response: {e}"));
                }
            }
        }
        OperationResult::Failed { error } => {
            out.status = "failed".into();
            out.error = Some(error)
        }
        OperationResult::Canceled { error } => {
            out.status = "canceled".into();
            out.error = error
        }
        OperationResult::DeadlineExceeded => out.status = "deadline_exceeded".into(),
        OperationResult::OutcomeUnknown { error } => {
            out.status = "outcome_unknown".into();
            out.error = Some(error)
        }
        OperationResult::Unsupported { operation } => {
            out.status = "unsupported".into();
            out.error = Some(operation)
        }
        OperationResult::Unauthorized => out.status = "unauthorized".into(),
    };
    out
}
pub(crate) fn required<T>(value: Option<T>, name: &str) -> Result<T, ToolError> {
    value.ok_or_else(|| ToolError::invalid(format!("{name} is required")))
}
pub(crate) fn parse_uuid(value: Option<String>, name: &str) -> Result<Uuid, ToolError> {
    Uuid::parse_str(&required(value, name)?)
        .map_err(|_| ToolError::invalid(format!("{name} must be a UUID")))
}
/// Accept stable IDs or unique human-friendly names. Never mistake an unknown name for
/// an offline device. Name matching is case-insensitive only after exact matching.
pub(crate) fn select_device<'a>(
    records: &'a [DeviceRecord],
    selector: &str,
) -> Result<&'a DeviceRecord, DispatchError> {
    let key = selector.trim();
    if key.is_empty() {
        return Err(DispatchError::NotFound(selector.to_owned()));
    }
    if let Some(record) = records.iter().find(|d| d.device_id == key) {
        return if record.revoked {
            Err(DispatchError::Revoked(record.device_name.clone()))
        } else {
            Ok(record)
        };
    }
    let exact: Vec<_> = records
        .iter()
        .filter(|d| d.device_name == key || d.display_name.as_deref() == Some(key))
        .collect();
    let matches: Vec<_> = if exact.is_empty() {
        records
            .iter()
            .filter(|d| {
                d.device_name.eq_ignore_ascii_case(key)
                    || d.display_name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(key))
            })
            .collect()
    } else {
        exact
    };
    match matches.as_slice() {
        [record] if record.revoked => Err(DispatchError::Revoked(record.device_name.clone())),
        [record] => Ok(record),
        [] => Err(DispatchError::NotFound(selector.to_owned())),
        _ => Err(DispatchError::Ambiguous {
            selector: selector.to_owned(),
            matches: matches
                .iter()
                .map(|d| format!("{} ({})", d.device_name, d.device_id))
                .collect(),
        }),
    }
}

pub(crate) async fn resolve_device(
    state: &AppState,
    selector: &str,
) -> Result<String, DispatchError> {
    // Common paths need no registry lookup; the direct online UUID is canonical.
    if state.devices.read().await.contains_key(selector) {
        return Ok(selector.to_owned());
    }
    let records = state
        .store
        .list_devices()
        .map_err(|err| DispatchError::Storage(err.to_string()))?;
    Ok(select_device(&records, selector)?.device_id.clone())
}

pub(crate) fn dispatch_error(error: DispatchError) -> String {
    match error {
        DispatchError::NotFound(name) => format!(
            "device_not_found: no enrolled device matches '{name}'; use devices to see valid names"
        ),
        DispatchError::Ambiguous { selector, matches } => format!(
            "ambiguous_device: '{selector}' matches {}; select a device ID",
            matches.join(", ")
        ),
        DispatchError::Revoked(name) => {
            format!("device_revoked: '{name}' has been revoked and must be re-enrolled")
        }
        DispatchError::Offline(name) => format!(
            "device_offline: '{name}' is enrolled but is not connected; it will reconnect when reachable"
        ),
        DispatchError::Unsupported(message) => format!("unsupported: {message}"),
        DispatchError::Storage(error) => format!("device_registry_error: {error}"),
        DispatchError::ConnectionClosed => {
            "device_connection_closed: device disconnected while sending the request".into()
        }
        DispatchError::QueueBusy => {
            "agent_busy: request deadline expired before dispatch; operation was not sent and may be retried".into()
        }
    }
}

pub(crate) async fn dispatch_to_device(
    state: &AppState,
    selector: String,
    operation: AgentOperation,
    timeout_ms: u64,
) -> Result<AgentResponse, DispatchError> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    let device_id = resolve_device(state, &selector).await?;
    let (tx, retire, generation) = {
        let devices = state.devices.read().await;
        let device = devices
            .get(&device_id)
            .ok_or_else(|| DispatchError::Offline(selector.clone()))?;
        if let AgentOperation::Files(FileRequest {
            action: FileAction::List { offset, length, .. },
        }) = &operation
            && (offset.is_some() || length.is_some())
            && !device
                .hello
                .capabilities
                .iter()
                .any(|capability| capability == FILE_LIST_PAGINATION_CAPABILITY)
        {
            return Err(DispatchError::Unsupported(
                "this connection does not advertise files.list_page; update the device agent for directory pagination, or omit offset/length for its legacy bounded listing".into(),
            ));
        }
        (device.tx.clone(), device.retire.clone(), device.generation)
    };
    // Reserve before registering a waiter: cancellation or timeout while queued
    // cannot leak pending state or enqueue a command after its caller gives up.
    let permit = match tokio::time::timeout_at(deadline, tx.reserve()).await {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => {
            let _ = retire.try_send(());
            return Err(DispatchError::ConnectionClosed);
        }
        Err(_) => return Err(DispatchError::QueueBusy),
    };
    let id = Uuid::new_v4();
    let (_pending, reply_rx) = state.pending.register(id, device_id.clone(), generation);
    permit.send(GatewayToAgent::Request(AgentRequest { id, operation }));
    let error = match tokio::time::timeout_at(deadline, reply_rx).await {
        Ok(Ok(response)) => return Ok(response),
        Ok(Err(_)) => "device connection closed after dispatch; operation was not replayed",
        Err(_) => "gateway timed out waiting for device response; operation was not replayed",
    };
    // The operation may still complete. Liveness belongs to the independent
    // heartbeat/socket checks; an operation deadline alone must not retire it.
    warn!(device_id=%device_id, request_id=%id, timeout_ms,
        "agent operation response unavailable; outcome unknown, no replay");
    Ok(AgentResponse {
        id,
        result: OperationResult::OutcomeUnknown {
            error: error.into(),
        },
    })
}
