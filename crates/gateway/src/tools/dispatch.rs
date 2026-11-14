use crate::*;

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
    permit.send(Outbound::Json(GatewayToAgent::Request(AgentRequest {
        id,
        operation,
    })));
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
