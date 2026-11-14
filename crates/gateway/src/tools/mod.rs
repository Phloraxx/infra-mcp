use super::*;

mod containers;
mod copy;
mod desktop;
mod devices;
mod dispatch;
mod exec;
mod files;
mod git;
mod jobs;
mod network;
mod processes;
mod search;
mod services;
mod system;
mod terminal;
mod transfer;

pub(crate) use containers::*;
pub(crate) use copy::*;
pub(crate) use desktop::*;
pub(crate) use devices::*;
pub(crate) use dispatch::*;
pub(crate) use exec::*;
pub(crate) use files::*;
pub(crate) use git::*;
pub(crate) use jobs::*;
pub(crate) use network::*;
pub(crate) use processes::*;
pub(crate) use search::*;
pub(crate) use services::*;
pub(crate) use system::*;
pub(crate) use terminal::*;
pub(crate) use transfer::*;

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
        Ok(McpJson(crate::copy::run(&self.state, input).await?))
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
