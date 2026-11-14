//! A bounded, resumable relay between two existing signed agents.
//! No local files or additional transport: each chunk goes through authenticated RPC.
//!
//! The choreography lives here once and is driven by two callers: the synchronous
//! `copy` tool (a bounded slice per MCP call) and the durable queue in
//! [`crate::copy_queue`] (one task per job, progress persisted between chunks).
//! Both use the same two transfer IDs, so a job started by one can be resumed by
//! the other.
use super::*;
use serde_json::{Value, json};
use sha2::Digest;

const CHUNK: usize = 256 * 1024;
const DEFAULT_BUDGET: u64 = 16 * 1024 * 1024;
const MAX_BUDGET: u64 = 64 * 1024 * 1024;
const MAX_SECONDS: u64 = 100;
const RPC_TIMEOUT_MS: u64 = 20_000;

/// Why a relay step stopped. The distinction is what lets the queue pause a job
/// instead of failing it: an endpoint that went away is not a broken copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RelayError {
    /// A device is not connected (or its connection closed). Retry when it is back.
    Offline(String),
    /// The chunk may or may not have landed. Never replayed blindly: the next
    /// attempt re-reads the destination's confirmed offset first.
    Uncertain(String),
    /// A real failure: bad paths, a mismatched transfer, a checksum mismatch.
    Failed(String),
}

impl RelayError {
    pub(super) fn message(&self) -> &str {
        match self {
            Self::Offline(message) | Self::Uncertain(message) | Self::Failed(message) => message,
        }
    }
}

impl From<String> for RelayError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

/// Which side of the relay an RPC went to. An uncertain chunk names it in the
/// pause reason, so an operator can tell whether the source or the destination
/// went away without reading the transfer IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Endpoint {
    Source,
    Destination,
}

impl Endpoint {
    fn role(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Destination => "destination",
        }
    }
}

/// Everything one relay needs to keep going. `source_id`/`destination_id` are
/// filled in as [`begin`] progresses, so a caller that fails halfway still knows
/// which transfers exist and can resume or cancel them.
#[derive(Debug, Clone, Default)]
pub(super) struct Relay {
    pub(super) from: String,
    pub(super) to: String,
    pub(super) source_id: Option<Uuid>,
    pub(super) destination_id: Option<Uuid>,
    pub(super) offset: u64,
    pub(super) size: u64,
    pub(super) sha256: String,
    pub(super) source_binary: bool,
    pub(super) destination_binary: bool,
}

pub(super) struct BeginRequest<'a> {
    pub(super) from: &'a str,
    pub(super) to: &'a str,
    pub(super) source_path: &'a str,
    pub(super) destination_path: &'a str,
    pub(super) overwrite: bool,
    pub(super) create_parents: bool,
    pub(super) source_id: Option<Uuid>,
    pub(super) destination_id: Option<Uuid>,
}

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

/// Classify a dispatch failure. A device that is not connected is a pause, not a
/// failure; everything else means the request itself cannot succeed as written.
fn classify_dispatch(error: DispatchError) -> RelayError {
    match error {
        DispatchError::Offline(_) | DispatchError::ConnectionClosed | DispatchError::QueueBusy => {
            RelayError::Offline(dispatch_error(error))
        }
        other => RelayError::Failed(dispatch_error(other)),
    }
}

async fn step(
    state: &AppState,
    device: &str,
    endpoint: Endpoint,
    action: TransferAction,
) -> Result<Value, RelayError> {
    let response = dispatch_to_device(
        state,
        device.to_owned(),
        AgentOperation::Transfer(TransferRequest { action }),
        RPC_TIMEOUT_MS,
    )
    .await
    .map_err(classify_dispatch)?;
    body_of(response, endpoint)
}

fn body_of(response: AgentResponse, endpoint: Endpoint) -> Result<Value, RelayError> {
    let role = endpoint.role();
    match response.result {
        OperationResult::Ok { body } => Ok(body),
        OperationResult::Failed { error } => Err(RelayError::Failed(error)),
        OperationResult::Canceled { error } => Err(RelayError::Failed(
            error.unwrap_or_else(|| "transfer canceled".into()),
        )),
        OperationResult::DeadlineExceeded => Err(RelayError::Uncertain(format!(
            "transfer deadline exceeded; inspect status before retrying ({role} endpoint)"
        ))),
        OperationResult::OutcomeUnknown { error } => Err(RelayError::Uncertain(format!(
            "transfer outcome unknown (do not assume chunk failed); resume with the returned transfer IDs: {error} ({role} endpoint)"
        ))),
        OperationResult::Unsupported { operation } => Err(RelayError::Failed(format!(
            "transfer unsupported: {operation}"
        ))),
        OperationResult::Unauthorized => Err(RelayError::Failed("transfer unauthorized".into())),
    }
}

/// Resolve (or create) both transfers and report where the destination stands.
///
/// The destination's own `bytes_transferred` is the resume offset: it is the only
/// party that knows which chunks actually landed, so a restart never trusts a
/// stored counter over it. On failure the source transfer is canceled when no
/// upload exists yet, so a failed initialization does not leave a source-only
/// "ready" transfer listed as active work. Both IDs are preserved once an upload
/// exists, so retries can safely resume after uncertain chunk outcomes.
pub(super) async fn begin(
    state: &AppState,
    request: BeginRequest<'_>,
    relay: &mut Relay,
) -> Result<(), RelayError> {
    relay.from = request.from.to_owned();
    relay.to = request.to.to_owned();
    relay.source_id = request.source_id;
    relay.destination_id = request.destination_id;
    // Each side chooses independently: a binary-capable source can feed a
    // JSON-only destination and the other way round.
    relay.source_binary = supports_capability(
        state,
        request.from,
        infra_protocol::TRANSFER_BINARY_CAPABILITY,
    )
    .await;
    relay.destination_binary = supports_capability(
        state,
        request.to,
        infra_protocol::TRANSFER_BINARY_CAPABILITY,
    )
    .await;
    let outcome = begin_inner(state, request, relay).await;
    if outcome.is_err()
        && relay.destination_id.is_none()
        && let Some(id) = relay.source_id
    {
        cancel_transfer(state, &relay.from, Endpoint::Source, id).await;
    }
    outcome
}

async fn begin_inner(
    state: &AppState,
    request: BeginRequest<'_>,
    relay: &mut Relay,
) -> Result<(), RelayError> {
    // Resume existing transfer metadata instead of starting a new upload.
    let download = if let Some(id) = relay.source_id {
        step(
            state,
            request.from,
            Endpoint::Source,
            TransferAction::Status { transfer_id: id },
        )
        .await?
    } else {
        step(
            state,
            request.from,
            Endpoint::Source,
            TransferAction::DownloadBegin {
                path: request.source_path.to_owned(),
            },
        )
        .await?
    };
    let source = meta(&download)?;
    if text_field(source, "path")? != request.source_path
        || text_field(source, "direction")? != "download"
    {
        return Err(RelayError::Failed(
            "source transfer ID does not match the requested source".into(),
        ));
    }
    relay.source_id = Some(transfer_id(source)?);
    relay.size = num_field(source, "size")?;
    relay.sha256 = text_field(source, "sha256")?.to_owned();

    let upload = if let Some(id) = relay.destination_id {
        step(
            state,
            request.to,
            Endpoint::Destination,
            TransferAction::Status { transfer_id: id },
        )
        .await?
    } else {
        step(
            state,
            request.to,
            Endpoint::Destination,
            TransferAction::UploadBegin {
                path: request.destination_path.to_owned(),
                size: relay.size,
                sha256: Some(relay.sha256.clone()),
                overwrite: request.overwrite,
                create_parents: request.create_parents,
            },
        )
        .await?
    };
    let dest = meta(&upload)?;
    if text_field(dest, "path")? != request.destination_path
        || text_field(dest, "direction")? != "upload"
        || num_field(dest, "size")? != relay.size
        || text_field(dest, "sha256")? != relay.sha256
    {
        return Err(RelayError::Failed(
            "destination transfer does not match requested path/size/checksum".into(),
        ));
    }
    relay.destination_id = Some(transfer_id(dest)?);
    if text_field(dest, "status")? == "completed" {
        relay.offset = relay.size;
        return Ok(());
    }
    if text_field(dest, "status")? != "uploading" {
        return Err(RelayError::Failed(
            "destination transfer is not an active upload".into(),
        ));
    }
    relay.offset = num_field(dest, "bytes_transferred")?;
    if relay.offset > relay.size {
        return Err(RelayError::Failed(
            "destination offset exceeds declared size".into(),
        ));
    }
    Ok(())
}

/// Relay exactly one chunk from `offset` and return the destination's confirmed
/// next offset. `max_len` bounds the chunk (the synchronous path clamps it to its
/// per-call budget); the wire chunk size is always the smaller of that and
/// [`CHUNK`]. The bytes are folded into `hasher` so a caller can prove the
/// relayed stream matches the source digest.
pub(super) async fn relay_chunk(
    state: &AppState,
    relay: &Relay,
    offset: u64,
    max_len: usize,
    hasher: &mut Sha256,
) -> Result<u64, RelayError> {
    let source_id = relay.source_id.expect("begin sets the source transfer");
    let destination_id = relay
        .destination_id
        .expect("begin sets the destination transfer");
    let count = (relay.size - offset).min(max_len as u64).min(CHUNK as u64) as usize;
    let (chunk, payload) = if relay.source_binary {
        let (response, frame) = download_chunk_binary(
            state,
            &relay.from,
            source_id,
            offset,
            Some(count),
            RPC_TIMEOUT_MS,
        )
        .await
        .map_err(classify_dispatch)?;
        (body_of(response, Endpoint::Source)?, frame)
    } else {
        (
            step(
                state,
                &relay.from,
                Endpoint::Source,
                TransferAction::DownloadChunk {
                    transfer_id: source_id,
                    offset,
                    length: Some(count),
                },
            )
            .await?,
            None,
        )
    };
    if num_field(&chunk, "offset")? != offset {
        return Err(RelayError::Failed(
            "download offset mismatch; refusing to write".into(),
        ));
    }
    let next = num_field(&chunk, "next_offset")?;
    if next <= offset || next > relay.size || next - offset > count as u64 {
        return Err(RelayError::Failed(
            "download chunk size/offset invalid".into(),
        ));
    }
    // The binary path hands the bytes straight to the destination; the JSON path
    // decodes the base64 the agent produced.
    let bytes = match payload {
        Some(frame) => {
            if frame.payload.len() as u64 != next - offset {
                return Err(RelayError::Failed(
                    "bulk chunk length does not match the reported offset range".into(),
                ));
            }
            frame.payload
        }
        None => {
            let encoded = text_field(&chunk, "data_base64")?.to_owned();
            if encoded.is_empty() {
                return Err(RelayError::Failed(
                    "unexpected empty chunk before end of source".into(),
                ));
            }
            STANDARD
                .decode(encoded)
                .map_err(|_| RelayError::Failed("download chunk is not valid base64".into()))?
        }
    };
    if bytes.is_empty() {
        return Err(RelayError::Failed(
            "unexpected empty chunk before end of source".into(),
        ));
    }
    hasher.update(&bytes);
    let written = if relay.destination_binary {
        body_of(
            upload_chunk_binary(
                state,
                &relay.to,
                destination_id,
                offset,
                &bytes,
                RPC_TIMEOUT_MS,
            )
            .await
            .map_err(classify_dispatch)?,
            Endpoint::Destination,
        )?
    } else {
        step(
            state,
            &relay.to,
            Endpoint::Destination,
            TransferAction::UploadChunk {
                transfer_id: destination_id,
                offset,
                data_base64: STANDARD.encode(&bytes),
            },
        )
        .await?
    };
    if num_field(&written, "next_offset")? != next {
        return Err(RelayError::Failed(
            "destination upload offset mismatch; inspect status before continuing".into(),
        ));
    }
    Ok(next)
}

/// Commit the upload and verify the destination's whole-file digest.
pub(super) async fn commit(state: &AppState, relay: &Relay) -> Result<(), RelayError> {
    let destination_id = relay
        .destination_id
        .expect("begin sets the destination transfer");
    let commit = step(
        state,
        &relay.to,
        Endpoint::Destination,
        TransferAction::UploadCommit {
            transfer_id: destination_id,
        },
    )
    .await?;
    if text_field(&commit, "sha256")? != relay.sha256 {
        return Err(RelayError::Failed(
            "destination checksum mismatch after commit".into(),
        ));
    }
    Ok(())
}

/// Best-effort cancel of one transfer. Used for cleanup, so a failure here must
/// not mask the error that triggered it.
pub(super) async fn cancel_transfer(state: &AppState, device: &str, endpoint: Endpoint, id: Uuid) {
    let _ = step(
        state,
        device,
        endpoint,
        TransferAction::Cancel { transfer_id: id },
    )
    .await;
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
    let source_id = input
        .source_transfer_id
        .as_ref()
        .map(|s| Uuid::parse_str(s).map_err(|_| "invalid source_transfer_id UUID"))
        .transpose()?;
    let destination_id = input
        .destination_transfer_id
        .as_ref()
        .map(|s| Uuid::parse_str(s).map_err(|_| "invalid destination_transfer_id UUID"))
        .transpose()?;
    let mut relay = Relay::default();
    let request = BeginRequest {
        from: &from,
        to: &to,
        source_path: &input.source_path,
        destination_path: &input.destination_path,
        overwrite: input.overwrite.unwrap_or(false),
        create_parents: input.create_parents.unwrap_or(false),
        source_id,
        destination_id,
    };

    let operation = async {
        begin(state, request, &mut relay).await?;
        let start_offset = relay.offset;
        let mut hasher = Sha256::new();
        while relay.offset < relay.size
            && relay.offset - start_offset < budget
            && started.elapsed() < Duration::from_secs(MAX_SECONDS)
        {
            let remaining = budget - (relay.offset - start_offset);
            relay.offset =
                relay_chunk(state, &relay, relay.offset, remaining as usize, &mut hasher).await?;
        }
        if relay.offset == relay.size {
            commit(state, &relay).await?;
        }
        Ok::<(), RelayError>(())
    }
    .await;

    let (status, error) = match operation {
        Ok(()) if relay.offset == relay.size => ("completed", None),
        Ok(()) => ("in_progress", None),
        Err(error) => ("failed", Some(error.message().to_owned())),
    };
    let mut value = progress(
        status,
        &from,
        &to,
        &input,
        relay.source_id,
        relay.destination_id,
        relay.offset,
        relay.size,
        &relay.sha256,
        error,
    );
    // Which wire path each side used, so a benchmark or an operator can tell a
    // binary relay from the base64 fallback without reading logs.
    value["transport"] = json!({
        "source": if relay.source_binary { "binary" } else { "json" },
        "destination": if relay.destination_binary { "binary" } else { "json" },
    });
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

    #[test]
    fn dispatch_failures_are_classified_as_pause_or_failure() {
        assert!(matches!(
            classify_dispatch(DispatchError::Offline("laptop".into())),
            RelayError::Offline(_)
        ));
        assert!(matches!(
            classify_dispatch(DispatchError::ConnectionClosed),
            RelayError::Offline(_)
        ));
        assert!(matches!(
            classify_dispatch(DispatchError::QueueBusy),
            RelayError::Offline(_)
        ));
        assert!(matches!(
            classify_dispatch(DispatchError::NotFound("ghost".into())),
            RelayError::Failed(_)
        ));
        assert!(matches!(
            classify_dispatch(DispatchError::Protocol("bad frame".into())),
            RelayError::Failed(_)
        ));
    }

    #[test]
    fn an_uncertain_chunk_is_not_a_failure() {
        let uncertain = body_of(
            AgentResponse {
                id: Uuid::new_v4(),
                result: OperationResult::OutcomeUnknown {
                    error: "socket closed".into(),
                },
            },
            Endpoint::Destination,
        )
        .unwrap_err();
        assert!(matches!(uncertain, RelayError::Uncertain(_)));
        // The pause reason keeps its structure and names the side the chunk was
        // heading to, so an operator can tell which endpoint went away.
        assert_eq!(
            uncertain.message(),
            "transfer outcome unknown (do not assume chunk failed); resume with the returned transfer IDs: socket closed (destination endpoint)"
        );
        let source_side = body_of(
            AgentResponse {
                id: Uuid::new_v4(),
                result: OperationResult::DeadlineExceeded,
            },
            Endpoint::Source,
        )
        .unwrap_err();
        assert!(
            source_side.message().ends_with("(source endpoint)"),
            "{}",
            source_side.message()
        );
        let failed = body_of(
            AgentResponse {
                id: Uuid::new_v4(),
                result: OperationResult::Failed {
                    error: "no such file".into(),
                },
            },
            Endpoint::Source,
        )
        .unwrap_err();
        assert!(matches!(failed, RelayError::Failed(_)));
        assert_eq!(failed.message(), "no such file");
    }
}
