//! Capability-gated bulk frames on the agent link.
//!
//! Control stays JSON. A payload-bearing operation is announced with a normal
//! [`GatewayToAgent::Request`] and the bytes follow in one [`BulkFrame`] on the
//! same socket, so the request/response correlation, the pending registry and
//! the audit trail are unchanged. Devices that did not advertise
//! `transfer.binary_v1` / `terminal.binary_v1` keep the base64 JSON path.
use super::*;
use infra_protocol::{BulkFrame, BulkKind, OwnedBulkFrame};

/// One frame the connection writer must put on the socket. JSON control frames
/// and bulk payload frames share one channel so an announcement always precedes
/// its payload.
// The channel already carried `GatewayToAgent` before bulk frames existed, so
// the enum is no larger than the message type it replaced.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Outbound {
    Json(GatewayToAgent),
    Binary(Vec<u8>),
}

impl From<GatewayToAgent> for Outbound {
    fn from(message: GatewayToAgent) -> Self {
        Self::Json(message)
    }
}

/// Per-connection registry of readers waiting for a bulk payload, keyed by the
/// transfer or terminal id in the frame header.
#[derive(Clone, Default)]
pub(crate) struct BulkWaiters(Arc<std::sync::Mutex<HashMap<Uuid, mpsc::Sender<OwnedBulkFrame>>>>);

/// Registration guard: dropping it removes the reader, so a late frame is never
/// delivered to a request that already gave up.
pub(crate) struct BulkWaiter {
    id: Uuid,
    registry: BulkWaiters,
}

impl BulkWaiters {
    pub(crate) fn register(
        &self,
        id: Uuid,
    ) -> Result<(BulkWaiter, mpsc::Receiver<OwnedBulkFrame>), String> {
        let (tx, rx) = mpsc::channel(2);
        let mut waiters = self.0.lock().expect("bulk registry poisoned");
        if waiters.contains_key(&id) {
            return Err(format!(
                "transfer {id} already has a bulk reader on this connection"
            ));
        }
        waiters.insert(id, tx);
        Ok((
            BulkWaiter {
                id,
                registry: self.clone(),
            },
            rx,
        ))
    }

    /// Hand a decoded frame to its reader. False means nobody is waiting, which
    /// is what a frame arriving after its request timed out looks like.
    pub(crate) fn deliver(&self, frame: OwnedBulkFrame) -> bool {
        let sender = self
            .0
            .lock()
            .expect("bulk registry poisoned")
            .get(&frame.id)
            .cloned();
        match sender {
            Some(sender) => sender.try_send(frame).is_ok(),
            None => false,
        }
    }

    /// Release every reader when the connection ends, so a relay blocked on a
    /// payload fails immediately instead of waiting for its deadline.
    pub(crate) fn clear(&self) {
        self.0.lock().expect("bulk registry poisoned").clear();
    }
}

impl Drop for BulkWaiter {
    fn drop(&mut self) {
        self.registry
            .0
            .lock()
            .expect("bulk registry poisoned")
            .remove(&self.id);
    }
}

/// True when the live connection for `device_id` advertised `capability`.
pub(crate) async fn supports_capability(
    state: &AppState,
    device_id: &str,
    capability: &str,
) -> bool {
    state
        .devices
        .read()
        .await
        .get(device_id)
        .is_some_and(|device| {
            device
                .hello
                .capabilities
                .iter()
                .any(|advertised| advertised == capability)
        })
}

fn outcome_unknown(id: Uuid, error: &str) -> AgentResponse {
    AgentResponse {
        id,
        result: OperationResult::OutcomeUnknown {
            error: error.into(),
        },
    }
}

/// Dispatch one request, optionally followed by a bulk payload, and collect the
/// response plus the payload the agent sends back for it.
async fn dispatch_bulk(
    state: &AppState,
    selector: String,
    operation: AgentOperation,
    bulk_id: Uuid,
    payload: Option<OwnedBulkFrame>,
    timeout_ms: u64,
) -> Result<(AgentResponse, Option<OwnedBulkFrame>), DispatchError> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    let device_id = resolve_device(state, &selector).await?;
    let (tx, retire, generation, waiters) = {
        let devices = state.devices.read().await;
        let device = devices
            .get(&device_id)
            .ok_or_else(|| DispatchError::Offline(selector.clone()))?;
        (
            device.tx.clone(),
            device.retire.clone(),
            device.generation,
            device.bulk.clone(),
        )
    };
    // Register before sending: the payload can arrive before this task is polled
    // again, and a frame with no reader would be dropped.
    let (waiter, mut bulk_rx) = waiters
        .register(bulk_id)
        .map_err(DispatchError::Unsupported)?;
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
    if let Some(frame) = payload {
        // The announcement is already queued, so the payload must follow it. If
        // it cannot be queued the agent keeps an announcement it will never
        // complete; the connection is the only thing that can clear it, which is
        // why the outcome is unknown rather than a clean failure.
        if tokio::time::timeout_at(deadline, tx.send(Outbound::Binary(frame.encode())))
            .await
            .is_err()
        {
            return Ok((
                outcome_unknown(
                    id,
                    "gateway could not queue the bulk payload; the chunk was not confirmed",
                ),
                None,
            ));
        }
    }
    let response = match tokio::time::timeout_at(deadline, reply_rx).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => {
            return Ok((
                outcome_unknown(
                    id,
                    "device connection closed after dispatch; operation was not replayed",
                ),
                None,
            ));
        }
        Err(_) => {
            return Ok((
                outcome_unknown(
                    id,
                    "gateway timed out waiting for device response; operation was not replayed",
                ),
                None,
            ));
        }
    };
    let wants_payload = matches!(
        &response.result,
        OperationResult::Ok { body }
            if body.get("data_binary").and_then(serde_json::Value::as_bool) == Some(true)
    );
    if !wants_payload {
        return Ok((response, None));
    }
    let frame = match tokio::time::timeout_at(deadline, bulk_rx.recv()).await {
        Ok(Some(frame)) => frame,
        Ok(None) => {
            return Ok((
                outcome_unknown(
                    id,
                    "device connection closed before the bulk payload arrived",
                ),
                None,
            ));
        }
        Err(_) => {
            return Ok((
                outcome_unknown(id, "gateway timed out waiting for the bulk payload"),
                None,
            ));
        }
    };
    drop(waiter);
    Ok((response, Some(frame)))
}

/// Send one upload chunk in a bulk frame. Only call this for a device that
/// advertised [`infra_protocol::TRANSFER_BINARY_CAPABILITY`].
pub(crate) async fn upload_chunk_binary(
    state: &AppState,
    device: &str,
    transfer_id: Uuid,
    offset: u64,
    data: &[u8],
    timeout_ms: u64,
) -> Result<AgentResponse, String> {
    let operation = AgentOperation::Transfer(TransferRequest {
        action: TransferAction::UploadChunkBinary {
            transfer_id,
            offset,
            length: data.len(),
        },
    });
    let frame = OwnedBulkFrame::new(BulkKind::UploadChunk, transfer_id, offset, data.to_vec());
    dispatch_bulk(
        state,
        device.to_owned(),
        operation,
        transfer_id,
        Some(frame),
        timeout_ms,
    )
    .await
    .map(|(response, _)| response)
    .map_err(dispatch_error)
}

/// Read one download chunk over a bulk frame. Only call this for a device that
/// advertised [`infra_protocol::TRANSFER_BINARY_CAPABILITY`].
pub(crate) async fn download_chunk_binary(
    state: &AppState,
    device: &str,
    transfer_id: Uuid,
    offset: u64,
    length: Option<usize>,
    timeout_ms: u64,
) -> Result<(AgentResponse, Option<OwnedBulkFrame>), String> {
    let operation = AgentOperation::Transfer(TransferRequest {
        action: TransferAction::DownloadChunk {
            transfer_id,
            offset,
            length,
        },
    });
    let (response, frame) = dispatch_bulk(
        state,
        device.to_owned(),
        operation,
        transfer_id,
        None,
        timeout_ms,
    )
    .await
    .map_err(dispatch_error)?;
    if let Some(frame) = &frame
        && (frame.kind != BulkKind::DownloadChunk
            || frame.id != transfer_id
            || frame.offset != offset)
    {
        return Err("device returned a bulk frame that does not match the requested chunk".into());
    }
    Ok((response, frame))
}

/// Write terminal input in a bulk frame. Only call this for a device that
/// advertised [`infra_protocol::TERMINAL_BINARY_CAPABILITY`].
pub(crate) async fn terminal_write_binary(
    state: &AppState,
    device: &str,
    session_id: Uuid,
    data: &[u8],
    timeout_ms: u64,
) -> Result<AgentResponse, String> {
    let operation = AgentOperation::Terminal(TerminalRequest {
        action: TerminalAction::WriteBinary {
            session_id,
            length: data.len(),
        },
    });
    let frame = OwnedBulkFrame::new(BulkKind::TerminalData, session_id, 0, data.to_vec());
    dispatch_bulk(
        state,
        device.to_owned(),
        operation,
        session_id,
        Some(frame),
        timeout_ms,
    )
    .await
    .map(|(response, _)| response)
    .map_err(dispatch_error)
}

/// Decode a frame received from an agent and hand it to its reader.
pub(crate) fn deliver_frame(waiters: &BulkWaiters, bytes: &[u8]) -> Result<bool, String> {
    let frame = BulkFrame::decode(bytes).map_err(|error| error.to_string())?;
    Ok(waiters.deliver(frame.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_is_removed_when_it_gives_up() {
        let waiters = BulkWaiters::default();
        let id = Uuid::new_v4();
        let (waiter, mut rx) = waiters.register(id).expect("register");
        assert!(
            waiters.register(id).is_err(),
            "one transfer cannot have two readers"
        );
        let frame = OwnedBulkFrame::new(BulkKind::DownloadChunk, id, 0, vec![1, 2, 3]);
        assert!(waiters.deliver(frame.clone()));
        assert_eq!(rx.try_recv().expect("frame"), frame);
        drop(waiter);
        assert!(
            !waiters.deliver(frame),
            "a late frame must not be delivered after the reader gave up"
        );
    }

    #[test]
    fn clearing_releases_blocked_readers() {
        let waiters = BulkWaiters::default();
        let (_waiter, mut rx) = waiters.register(Uuid::new_v4()).expect("register");
        waiters.clear();
        assert!(rx.try_recv().is_err());
        assert!(rx.is_closed(), "the reader must observe the closed channel");
    }

    #[test]
    fn malformed_frames_are_rejected_before_delivery() {
        let waiters = BulkWaiters::default();
        assert!(deliver_frame(&waiters, b"not a bulk frame").is_err());
        let frame = BulkFrame::new(BulkKind::TerminalData, Uuid::nil(), 0, b"x").encode();
        assert_eq!(
            deliver_frame(&waiters, &frame),
            Ok(false),
            "a frame with no reader is not delivered"
        );
    }
}
