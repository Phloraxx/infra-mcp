//! A bounded, resumable relay between two existing signed agents.
//! No local files or additional transport: each chunk goes through authenticated RPC.
use super::*;
use serde_json::{Value, json};

const CHUNK: usize = 256 * 1024;
const DEFAULT_BUDGET: u64 = 16 * 1024 * 1024;
const MAX_BUDGET: u64 = 64 * 1024 * 1024;
const MAX_SECONDS: u64 = 100;
const RPC_TIMEOUT_MS: u64 = 20_000;

fn text_field<'a>(data: &'a Value, name: &str) -> Result<&'a str, String> {
    data.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("malformed transfer response: missing {name}"))
}
fn num_field(data: &Value, name: &str) -> Result<u64, String> {
    data.get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("malformed transfer response: missing {name}"))
}
fn transfer_id(data: &Value) -> Result<Uuid, String> {
    Uuid::parse_str(text_field(data, "id")?)
        .map_err(|_| "malformed transfer response: invalid ID".into())
}
fn meta(data: &Value) -> Result<&Value, String> {
    data.get("transfer")
        .ok_or_else(|| "malformed transfer response: missing transfer".into())
}
async fn step(state: &AppState, device: &str, action: TransferAction) -> Result<Value, String> {
    let response = dispatch_to_device(
        state,
        device.to_owned(),
        AgentOperation::Transfer(TransferRequest { action }),
        RPC_TIMEOUT_MS,
    )
    .await
    .map_err(dispatch_error)?;
    match response.result {
        OperationResult::Ok { body } => Ok(body),
        OperationResult::Failed { error } => Err(error),
        OperationResult::Canceled { error } => {
            Err(error.unwrap_or_else(|| "transfer canceled".into()))
        }
        OperationResult::DeadlineExceeded => {
            Err("transfer deadline exceeded; inspect status before retrying".into())
        }
        OperationResult::OutcomeUnknown { error } => Err(format!(
            "transfer outcome unknown (do not assume chunk failed); resume with the returned transfer IDs: {error}"
        )),
        OperationResult::Unsupported { operation } => {
            Err(format!("transfer unsupported: {operation}"))
        }
        OperationResult::Unauthorized => Err("transfer unauthorized".into()),
    }
}

#[allow(clippy::too_many_arguments)]
fn progress(
    status: &str,
    from: &str,
    to: &str,
    input: &McpCopyInput,
    source_id: Option<Uuid>,
    destination_id: Option<Uuid>,
    offset: u64,
    total: u64,
    digest: &str,
    error: Option<String>,
) -> Value {
    json!({
        "status":status,
        "source_device_id":from,
        "destination_device_id":to,
        "source_path":input.source_path,
        "destination_path":input.destination_path,
        "source_transfer_id":source_id,
        "destination_transfer_id":destination_id,
        "bytes_transferred":offset,
        "total_bytes":total,
        "sha256":digest,
        "error":error,
        "resume_hint":"Call copy again with the same paths/devices and both returned transfer IDs; inspect destination transfer status if outcome is unknown."
    })
}

/// Processes at most 64 MiB/100 seconds per call to avoid tying up one MCP request.
/// Returns stable transfer IDs on partial progress; never automatically replays an uncertain chunk.
pub(super) async fn run(state: &AppState, input: McpCopyInput) -> Result<Value, String> {
    if input.source_path.trim().is_empty() || input.destination_path.trim().is_empty() {
        return Err("source_path and destination_path must be nonempty".into());
    }
    if input.source_transfer_id.is_some() != input.destination_transfer_id.is_some() {
        return Err("resume requires both source_transfer_id and destination_transfer_id".into());
    }
    let from = resolve_device(state, &input.source_device)
        .await
        .map_err(dispatch_error)?;
    let to = resolve_device(state, &input.destination_device)
        .await
        .map_err(dispatch_error)?;
    if from == to && input.source_path == input.destination_path {
        return Err("source and destination cannot be the same file on the same device".into());
    }
    let budget = input
        .max_bytes
        .unwrap_or(DEFAULT_BUDGET)
        .clamp(CHUNK as u64, MAX_BUDGET);
    let started = Instant::now();
    let mut source_id = input
        .source_transfer_id
        .as_ref()
        .map(|s| Uuid::parse_str(s).map_err(|_| "invalid source_transfer_id UUID"))
        .transpose()?;
    let mut destination_id = input
        .destination_transfer_id
        .as_ref()
        .map(|s| Uuid::parse_str(s).map_err(|_| "invalid destination_transfer_id UUID"))
        .transpose()?;
    let mut offset = 0;
    let mut size = 0;
    let mut hash = String::new();

    let operation = async {
        // Resume existing transfer metadata instead of starting a new upload.
        let download = if let Some(id) = source_id {
            step(state, &from, TransferAction::Status { transfer_id: id }).await?
        } else {
            step(
                state,
                &from,
                TransferAction::DownloadBegin {
                    path: input.source_path.clone(),
                },
            )
            .await?
        };
        let source = meta(&download)?;
        if text_field(source, "path")? != input.source_path
            || text_field(source, "direction")? != "download"
        {
            return Err("source transfer ID does not match the requested source".into());
        }
        source_id = Some(transfer_id(source)?);
        size = num_field(source, "size")?;
        hash = text_field(source, "sha256")?.to_owned();

        let upload = if let Some(id) = destination_id {
            step(state, &to, TransferAction::Status { transfer_id: id }).await?
        } else {
            step(
                state,
                &to,
                TransferAction::UploadBegin {
                    path: input.destination_path.clone(),
                    size,
                    sha256: Some(hash.clone()),
                    overwrite: input.overwrite.unwrap_or(false),
                    create_parents: input.create_parents.unwrap_or(false),
                },
            )
            .await?
        };
        let dest = meta(&upload)?;
        if text_field(dest, "path")? != input.destination_path
            || text_field(dest, "direction")? != "upload"
            || num_field(dest, "size")? != size
            || text_field(dest, "sha256")? != hash
        {
            return Err("destination transfer does not match requested path/size/checksum".into());
        }
        destination_id = Some(transfer_id(dest)?);
        if text_field(dest, "status")? == "completed" {
            offset = size;
            return Ok(());
        }
        if text_field(dest, "status")? != "uploading" {
            return Err("destination transfer is not an active upload".into());
        }
        offset = num_field(dest, "bytes_transferred")?;
        if offset > size {
            return Err("destination offset exceeds declared size".into());
        }

        let start_offset = offset;
        while offset < size
            && offset - start_offset < budget
            && started.elapsed() < Duration::from_secs(MAX_SECONDS)
        {
            let count =
                (size - offset).min((budget - (offset - start_offset)).min(CHUNK as u64)) as usize;
            let chunk = step(
                state,
                &from,
                TransferAction::DownloadChunk {
                    transfer_id: source_id.expect("set above"),
                    offset,
                    length: Some(count),
                },
            )
            .await?;
            if num_field(&chunk, "offset")? != offset {
                return Err("download offset mismatch; refusing to write".into());
            }
            let encoded = text_field(&chunk, "data_base64")?.to_owned();
            if encoded.is_empty() {
                return Err("unexpected empty chunk before end of source".into());
            }
            let next = num_field(&chunk, "next_offset")?;
            if next <= offset || next > size || next - offset > count as u64 {
                return Err("download chunk size/offset invalid".into());
            }
            let written = step(
                state,
                &to,
                TransferAction::UploadChunk {
                    transfer_id: destination_id.expect("set above"),
                    offset,
                    data_base64: encoded,
                },
            )
            .await?;
            if num_field(&written, "next_offset")? != next {
                return Err(
                    "destination upload offset mismatch; inspect status before continuing".into(),
                );
            }
            offset = next;
        }
        if offset == size {
            let commit = step(
                state,
                &to,
                TransferAction::UploadCommit {
                    transfer_id: destination_id.expect("set above"),
                },
            )
            .await?;
            if text_field(&commit, "sha256")? != hash {
                return Err("destination checksum mismatch after commit".into());
            }
        }
        Ok(())
    }
    .await;

    // A failed destination initialization should not leave a source-only "ready"
    // transfer permanently listed as active work. Preserve both IDs once an upload
    // exists so retries can safely resume after uncertain chunk outcomes.
    if operation.is_err()
        && destination_id.is_none()
        && let Some(id) = source_id
    {
        let _ = step(state, &from, TransferAction::Cancel { transfer_id: id }).await;
    }
    let (status, error) = match operation {
        Ok(()) if offset == size => ("completed", None),
        Ok(()) => ("in_progress", None),
        Err(error) => ("failed", Some(error)),
    };
    let value = progress(
        status,
        &from,
        &to,
        &input,
        source_id,
        destination_id,
        offset,
        size,
        &hash,
        error,
    );
    let _ = state
        .store
        .record_activity(Some(&to), "device.copy", status, None);
    state.emit("activity", Some(to));
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_chunks_and_response_validation() {
        assert_eq!(DEFAULT_BUDGET, 16 * 1024 * 1024);
        assert_eq!(MAX_BUDGET, 64 * 1024 * 1024);
        assert!(text_field(&json!({}), "id").is_err());
        assert!(num_field(&json!({"size":"9"}), "size").is_err());
        assert!(transfer_id(&json!({"id":"not-uuid"})).is_err());
    }
}
