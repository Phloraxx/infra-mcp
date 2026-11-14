//! Durable cross-device copy queue.
//!
//! The synchronous `copy` tool relays at most 64 MiB per MCP call, so a 1 GiB
//! file means dozens of manual calls. This module keeps the same choreography
//! (the two transfer IDs in [`crate::copy`]) but drives it from one gateway task
//! per job, with the resume offset and both transfer IDs in SQLite.
//!
//! Crash safety rests on three rules:
//!
//! 1. The row is written before the action it describes (write-ahead), so a
//!    killed gateway leaves a row that under-claims progress, never over-claims.
//! 2. The destination's own `bytes_transferred` is the resume offset. It is the
//!    only party that knows which chunks landed, so a restart re-reads it instead
//!    of trusting the row.
//! 3. A terminal row is never resurrected: every driver write is guarded on the
//!    row not being terminal, so a cancel that races a chunk wins.
use super::*;
use crate::copy::{self, BeginRequest, Relay, RelayError};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use store::{CopyJob, CopyJobState, NewCopyJob};

/// How often the supervisor re-evaluates the table when nothing wakes it.
const SUPERVISOR_TICK: Duration = Duration::from_secs(3);
/// A paused job is retried no sooner than this, so an uncertain chunk cannot spin
/// the supervisor in a tight loop.
const PAUSE_RETRY: Duration = Duration::from_secs(2);

/// Coordination state shared by the supervisor, the drivers and the MCP tools.
/// It deliberately holds no `AppState`: the supervisor receives one when it is
/// spawned, which keeps the type constructible inside `AppState::new`.
#[derive(Clone, Default)]
pub(crate) struct CopyQueue {
    inner: Arc<CopyQueueInner>,
}

#[derive(Default)]
struct CopyQueueInner {
    running: Mutex<HashSet<String>>,
    cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
    wake: tokio::sync::Notify,
}

impl CopyQueue {
    /// Ask the supervisor to re-evaluate the table now (a device connected, a job
    /// was queued, a driver finished).
    pub(crate) fn wake(&self) {
        self.inner.wake.notify_one();
    }

    async fn notified(&self) {
        self.inner.wake.notified().await;
    }

    fn is_running(&self, id: &str) -> bool {
        self.inner
            .running
            .lock()
            .expect("copy queue poisoned")
            .contains(id)
    }

    /// Claim a job for this gateway. Returns the cancel flag the driver polls, or
    /// `None` when another task already owns it.
    fn begin(&self, id: &str) -> Option<Arc<AtomicBool>> {
        let mut running = self.inner.running.lock().expect("copy queue poisoned");
        if !running.insert(id.to_owned()) {
            return None;
        }
        let flag = Arc::new(AtomicBool::new(false));
        self.inner
            .cancel
            .lock()
            .expect("copy queue poisoned")
            .insert(id.to_owned(), flag.clone());
        Some(flag)
    }

    fn finish(&self, id: &str) {
        self.inner
            .running
            .lock()
            .expect("copy queue poisoned")
            .remove(id);
        self.inner
            .cancel
            .lock()
            .expect("copy queue poisoned")
            .remove(id);
    }

    /// Signal a live driver. False means no task is driving the job, which is
    /// fine: the row is already canceled and the supervisor will not pick it up.
    fn request_cancel(&self, id: &str) -> bool {
        self.inner
            .cancel
            .lock()
            .expect("copy queue poisoned")
            .get(id)
            .map(|flag| {
                flag.store(true, Ordering::Relaxed);
                true
            })
            .unwrap_or(false)
    }
}

/// Start the supervisor. It rebuilds the task list from the table on every wake
/// and every tick, so a gateway restart resumes whatever was running.
pub(crate) fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SUPERVISOR_TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = state.copy_queue.notified() => {}
                _ = ticker.tick() => {}
            }
            if let Err(error) = reconcile(&state).await {
                warn!(%error, "copy queue reconcile failed");
            }
        }
    });
}

/// Spawn a driver for every resumable job whose endpoints are both connected, and
/// pause the ones that are not.
async fn reconcile(state: &AppState) -> Result<(), StoreError> {
    let jobs = state.store.list_resumable_copy_jobs()?;
    if jobs.is_empty() {
        return Ok(());
    }
    let online: HashSet<String> = state.devices.read().await.keys().cloned().collect();
    for job in jobs {
        if state.copy_queue.is_running(&job.id) {
            continue;
        }
        let missing: Vec<&str> = [job.source_device.as_str(), job.dest_device.as_str()]
            .into_iter()
            .filter(|device| !online.contains(*device))
            .collect();
        if !missing.is_empty() {
            if job.state != CopyJobState::Paused {
                let reason = format!(
                    "paused: {} is offline; the job resumes when both endpoints are connected",
                    missing.join(" and ")
                );
                state
                    .store
                    .set_copy_job_state(&job.id, CopyJobState::Paused, Some(&reason))?;
                info!(job_id=%job.id, reason=%reason, "copy job paused");
            }
            continue;
        }
        // A job paused by an uncertain chunk must not be retried in a tight loop.
        if job.state == CopyJobState::Paused
            && now_ms().saturating_sub(job.updated_at_ms) < PAUSE_RETRY.as_millis() as u64
        {
            continue;
        }
        let Some(cancel) = state.copy_queue.begin(&job.id) else {
            continue;
        };
        let task_state = state.clone();
        let job_id = job.id.clone();
        tokio::spawn(async move {
            let _guard = RunningGuard {
                queue: task_state.copy_queue.clone(),
                id: job_id.clone(),
            };
            drive(&task_state, &job_id, cancel).await;
            task_state.copy_queue.wake();
        });
    }
    Ok(())
}

/// Releases the running claim when the driver task ends, including on a panic,
/// so a crashed driver cannot leave a job permanently claimed.
struct RunningGuard {
    queue: CopyQueue,
    id: String,
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.queue.finish(&self.id);
    }
}

enum JobOutcome {
    Completed,
    Paused(String),
    Failed(String),
    Canceled,
    /// The row reached a terminal state while this driver was working; leave it
    /// exactly as the other writer left it.
    AlreadyTerminal,
}

fn classify(error: RelayError) -> JobOutcome {
    match error {
        RelayError::Offline(message) => JobOutcome::Paused(format!("paused: {message}")),
        RelayError::Uncertain(message) => {
            JobOutcome::Paused(format!("paused after an uncertain chunk: {message}"))
        }
        RelayError::Failed(message) => JobOutcome::Failed(message),
    }
}

fn hex_digest(hasher: &Sha256) -> String {
    hasher
        .clone()
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn parse_id(value: &Option<String>) -> Option<Uuid> {
    value.as_deref().and_then(|id| Uuid::parse_str(id).ok())
}

async fn both_online(state: &AppState, from: &str, to: &str) -> bool {
    let devices = state.devices.read().await;
    devices.contains_key(from) && devices.contains_key(to)
}

async fn cancel_transfers(state: &AppState, relay: &Relay) {
    if let Some(id) = relay.destination_id {
        copy::cancel_transfer(state, &relay.to, copy::Endpoint::Destination, id).await;
    }
    if let Some(id) = relay.source_id {
        copy::cancel_transfer(state, &relay.from, copy::Endpoint::Source, id).await;
    }
}

/// Run one job to a terminal state or a pause, then record why.
async fn drive(state: &AppState, job_id: &str, cancel: Arc<AtomicBool>) {
    let job = match state.store.get_copy_job(job_id) {
        Ok(Some(job)) => job,
        Ok(None) => return,
        Err(error) => {
            warn!(job_id, %error, "failed to load a copy job");
            return;
        }
    };
    let outcome = run_job(state, &job, &cancel).await;
    let (next, error) = match outcome {
        JobOutcome::Completed => (CopyJobState::Completed, None),
        JobOutcome::Paused(reason) => (CopyJobState::Paused, Some(reason)),
        JobOutcome::Failed(reason) => (CopyJobState::Failed, Some(reason)),
        JobOutcome::Canceled => (
            CopyJobState::Canceled,
            Some("canceled by the owner".to_owned()),
        ),
        JobOutcome::AlreadyTerminal => return,
    };
    if let Err(failure) = state
        .store
        .set_copy_job_state(job_id, next, error.as_deref())
    {
        warn!(job_id, %failure, "failed to record the copy job outcome");
        return;
    }
    if next.is_terminal() {
        let _ = state.store.record_activity(
            Some(&job.dest_device),
            "copy.job",
            next.as_str(),
            Some(job_id),
        );
        state.emit("activity", Some(job.dest_device.clone()));
        info!(job_id, state = next.as_str(), "copy job finished");
    } else {
        info!(
            job_id,
            state = next.as_str(),
            reason = error.as_deref().unwrap_or_default(),
            "copy job paused"
        );
    }
}

async fn run_job(state: &AppState, job: &CopyJob, cancel: &AtomicBool) -> JobOutcome {
    // A terminal row is history, never work.
    if !job.state.is_resumable() {
        return JobOutcome::AlreadyTerminal;
    }
    // Write-ahead: the row says running before any transfer action is taken. A
    // false result means the row is already terminal, so this driver must stop.
    if !matches!(state.store.mark_copy_job_running(&job.id), Ok(true)) {
        return JobOutcome::AlreadyTerminal;
    }
    let mut relay = Relay::default();
    let request = BeginRequest {
        from: &job.source_device,
        to: &job.dest_device,
        source_path: &job.source_path,
        destination_path: &job.dest_path,
        // A queued copy is an explicit owner request to place this file at this
        // path, so it creates parents and replaces an existing destination. The
        // single-call `copy` keeps the finer-grained overwrite/create_parents
        // options for callers that need them.
        overwrite: true,
        create_parents: true,
        source_id: parse_id(&job.source_transfer_id),
        destination_id: parse_id(&job.dest_transfer_id),
    };
    if let Err(error) = copy::begin(state, request, &mut relay).await {
        return classify(error);
    }
    let (Some(source_id), Some(destination_id)) = (relay.source_id, relay.destination_id) else {
        return JobOutcome::Failed("relay did not produce both transfer IDs".into());
    };
    // Persist both IDs before the first chunk: a restart must resume the same two
    // transfers, not start a second upload next to the first.
    if !matches!(
        state.store.record_copy_job_transfers(
            &job.id,
            &source_id.to_string(),
            &destination_id.to_string(),
            relay.size,
            &relay.sha256,
        ),
        Ok(true)
    ) {
        // The row went terminal while this driver was setting up (a cancel that
        // raced the first chunk). Do not leave the transfers it just created
        // listed as active work on either agent.
        cancel_transfers(state, &relay).await;
        return JobOutcome::AlreadyTerminal;
    }
    // The destination is authoritative. A row that claims more than the
    // destination has means the destination lost confirmed bytes, which must not
    // be papered over by rewriting from a lower offset.
    if relay.offset < job.bytes_done {
        return JobOutcome::Failed(format!(
            "destination reports {} confirmed bytes but the job recorded {}; refusing to rewrite",
            relay.offset, job.bytes_done
        ));
    }
    let segment_start = relay.offset;
    let mut hasher = Sha256::new();
    let mut progress = relay.offset;
    if !matches!(
        state
            .store
            .record_copy_job_progress(&job.id, progress, progress, "", segment_start),
        Ok(true)
    ) {
        cancel_transfers(state, &relay).await;
        return JobOutcome::AlreadyTerminal;
    }
    if progress == relay.size {
        return finish(state, &relay, &hasher, segment_start).await;
    }
    loop {
        if cancel.load(Ordering::Relaxed) {
            cancel_transfers(state, &relay).await;
            return JobOutcome::Canceled;
        }
        if !both_online(state, &relay.from, &relay.to).await {
            return JobOutcome::Paused(format!(
                "paused: an endpoint went offline at {progress} of {} bytes",
                relay.size
            ));
        }
        // Write-ahead: record the offset about to be requested before acting.
        if !matches!(
            state.store.record_copy_job_progress(
                &job.id,
                progress,
                progress,
                &hex_digest(&hasher),
                segment_start,
            ),
            Ok(true)
        ) {
            cancel_transfers(state, &relay).await;
            return JobOutcome::AlreadyTerminal;
        }
        match copy::relay_chunk(state, &relay, progress, usize::MAX, &mut hasher).await {
            Ok(next) => {
                progress = next;
                if !matches!(
                    state.store.record_copy_job_progress(
                        &job.id,
                        progress,
                        progress,
                        &hex_digest(&hasher),
                        segment_start,
                    ),
                    Ok(true)
                ) {
                    cancel_transfers(state, &relay).await;
                    return JobOutcome::AlreadyTerminal;
                }
                if progress == relay.size {
                    return finish(state, &relay, &hasher, segment_start).await;
                }
            }
            Err(error) => return classify(error),
        }
    }
}

/// Commit the upload and, for a job that never resumed, prove the bytes this
/// gateway relayed hash to the digest the source declared.
async fn finish(
    state: &AppState,
    relay: &Relay,
    hasher: &Sha256,
    segment_start: u64,
) -> JobOutcome {
    if let Err(error) = copy::commit(state, relay).await {
        return classify(error);
    }
    if segment_start == 0 {
        let relayed = hex_digest(hasher);
        if relayed != relay.sha256 {
            return JobOutcome::Failed(format!(
                "relayed bytes hash to {relayed} but the source declared {}",
                relay.sha256
            ));
        }
    }
    JobOutcome::Completed
}

/// Queue a copy. Returns the job row so the caller can poll it immediately.
pub(crate) async fn start(
    state: &AppState,
    input: McpCopyStartInput,
) -> Result<serde_json::Value, String> {
    if input.source_path.trim().is_empty() || input.destination_path.trim().is_empty() {
        return Err("source_path and destination_path must be nonempty".into());
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
    let job = state
        .store
        .create_copy_job(NewCopyJob {
            source_device: &from,
            dest_device: &to,
            source_path: &input.source_path,
            dest_path: &input.destination_path,
        })
        .map_err(|error| format!("copy_queue_error: {error}"))?;
    state.copy_queue.wake();
    Ok(json!({"status": "queued", "job": job_view(&job)}))
}

/// One job by id, or the most recent jobs when no id is given.
pub(crate) fn status(
    state: &AppState,
    input: McpCopyStatusInput,
) -> Result<serde_json::Value, String> {
    if let Some(id) = input
        .job_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        let job = state
            .store
            .get_copy_job(id)
            .map_err(|error| format!("copy_queue_error: {error}"))?
            .ok_or_else(|| format!("copy_job_not_found: no copy job with id '{id}'"))?;
        return Ok(json!({"status": "ok", "job": job_view(&job)}));
    }
    let jobs = state
        .store
        .list_copy_jobs(input.limit.unwrap_or(20))
        .map_err(|error| format!("copy_queue_error: {error}"))?;
    Ok(json!({
        "status": "ok",
        "jobs": jobs.iter().map(job_view).collect::<Vec<_>>()
    }))
}

/// Cancel a job. A live driver stops before its next chunk and cancels both
/// transfers; a job with no driver is canceled in the table alone.
pub(crate) fn cancel(
    state: &AppState,
    input: McpCopyCancelInput,
) -> Result<serde_json::Value, String> {
    let id = input.job_id.trim();
    if id.is_empty() {
        return Err("job_id is required".into());
    }
    let job = state
        .store
        .cancel_copy_job(id)
        .map_err(|error| format!("copy_queue_error: {error}"))?
        .ok_or_else(|| format!("copy_job_not_found: no copy job with id '{id}'"))?;
    let signaled = state.copy_queue.request_cancel(id);
    state.copy_queue.wake();
    Ok(json!({
        "status": "ok",
        "cancel_signaled": signaled,
        "job": job_view(&job)
    }))
}

fn job_view(job: &CopyJob) -> serde_json::Value {
    let percent = if job.total_bytes == 0 {
        0.0
    } else {
        (job.bytes_done as f64 / job.total_bytes as f64) * 100.0
    };
    json!({
        "job_id": job.id,
        "state": job.state.as_str(),
        "source_device": job.source_device,
        "destination_device": job.dest_device,
        "source_path": job.source_path,
        "destination_path": job.dest_path,
        "bytes_done": job.bytes_done,
        "total_bytes": job.total_bytes,
        "percent": (percent * 10.0).round() / 10.0,
        "chunk_offset": job.chunk_offset,
        "sha256": job.sha256,
        "sha256_progress": job.sha256_progress,
        "sha256_progress_offset": job.sha256_progress_offset,
        "source_transfer_id": job.source_transfer_id,
        "destination_transfer_id": job.dest_transfer_id,
        "last_error": job.last_error,
        "created_at_unix_ms": job.created_at_ms,
        "updated_at_unix_ms": job.updated_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_state() -> (tempfile::TempDir, AppState) {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("infra.db")).unwrap();
        let state = AppState::new(store, None, None);
        (dir, state)
    }

    fn queue_job(state: &AppState) -> CopyJob {
        state
            .store
            .create_copy_job(NewCopyJob {
                source_device: "source-id",
                dest_device: "dest-id",
                source_path: "/source.bin",
                dest_path: "/destination.bin",
            })
            .unwrap()
    }

    #[tokio::test]
    async fn a_queued_job_pauses_while_an_endpoint_is_offline() {
        let (_dir, state) = test_state();
        let job = queue_job(&state);
        reconcile(&state).await.unwrap();
        let paused = state.store.get_copy_job(&job.id).unwrap().unwrap();
        assert_eq!(paused.state, CopyJobState::Paused);
        let reason = paused.last_error.expect("pause reason");
        assert!(reason.contains("offline"), "{reason}");
        assert!(reason.contains("source-id"), "{reason}");
        assert!(reason.contains("dest-id"), "{reason}");
        // Nothing was claimed, so a later reconcile can still drive it.
        assert!(!state.copy_queue.is_running(&job.id));
    }

    #[tokio::test]
    async fn a_terminal_job_is_never_claimed_again() {
        let (_dir, state) = test_state();
        let job = queue_job(&state);
        state
            .store
            .set_copy_job_state(&job.id, CopyJobState::Canceled, Some("canceled"))
            .unwrap();
        reconcile(&state).await.unwrap();
        assert!(!state.copy_queue.is_running(&job.id));
        assert_eq!(
            state.store.get_copy_job(&job.id).unwrap().unwrap().state,
            CopyJobState::Canceled
        );
    }

    #[tokio::test]
    async fn a_paused_job_waits_for_the_retry_window_before_it_is_claimed() {
        let (_dir, state) = test_state();
        let job = queue_job(&state);
        state
            .store
            .set_copy_job_state(&job.id, CopyJobState::Paused, Some("paused"))
            .unwrap();
        // Both endpoints are online, but the row was just paused: the supervisor
        // must not spin on it.
        {
            let mut devices = state.devices.write().await;
            for device in ["source-id", "dest-id"] {
                devices.insert(
                    device.to_owned(),
                    DeviceConnection {
                        hello: DeviceHello {
                            protocol: PROTOCOL_VERSION,
                            device_id: device.to_owned(),
                            device_name: device.to_owned(),
                            agent_version: "test".into(),
                            platform: "linux".into(),
                            arch: "aarch64".into(),
                            boot_id: "boot".into(),
                            agent_instance_id: Uuid::new_v4(),
                            capabilities: vec![],
                            metadata: Default::default(),
                        },
                        generation: 1,
                        tx: mpsc::channel(1).0,
                        retire: mpsc::channel(1).0,
                        bulk: BulkWaiters::default(),
                        reconcile: ReconcileSnapshot::default(),
                        reconcile_hash: None,
                    },
                );
            }
        }
        reconcile(&state).await.unwrap();
        assert!(
            !state.copy_queue.is_running(&job.id),
            "a freshly paused job must wait out the retry window"
        );
        // Age the row past the window: now it is claimed.
        let conn = state.store.connection().unwrap();
        conn.execute(
            "UPDATE copy_jobs SET updated_at_ms=?1 WHERE id=?2",
            rusqlite::params![(now_ms() - 60_000) as i64, job.id],
        )
        .unwrap();
        drop(conn);
        reconcile(&state).await.unwrap();
        assert!(state.copy_queue.is_running(&job.id));
        state.copy_queue.finish(&job.id);
    }

    #[test]
    fn cancel_signals_only_a_live_driver() {
        let queue = CopyQueue::default();
        assert!(!queue.request_cancel("job"));
        let flag = queue.begin("job").expect("claim");
        assert!(queue.is_running("job"));
        assert!(queue.begin("job").is_none(), "one driver per job");
        assert!(queue.request_cancel("job"));
        assert!(flag.load(Ordering::Relaxed));
        queue.finish("job");
        assert!(!queue.is_running("job"));
        assert!(!queue.request_cancel("job"));
    }

    #[test]
    fn a_job_view_reports_bounded_percentages() {
        let (_dir, state) = test_state();
        let job = queue_job(&state);
        let view = job_view(&job);
        assert_eq!(view["state"], "queued");
        assert_eq!(view["percent"], 0.0);
        assert_eq!(view["bytes_done"], 0);
        assert_eq!(view["job_id"], job.id);
    }
}
