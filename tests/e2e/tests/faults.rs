//! End-to-end fault-injection scenarios against the real gateway and agent.
//!
//! Each scenario maps to a row of the failure matrix in docs/NEXT_STEPS.md and
//! asserts observable state (store rows, audit events, response values), not
//! just the absence of a panic.
//!
//! The harness is Linux-only on purpose: it drives POSIX shell commands and PTYs
//! through the real agent, and the durable job/PTY workers are launched directly
//! on Linux. macOS submits those workers through launchd and Windows uses
//! ConPTY, so their acceptance stays a separate real-device gate rather than an
//! unverified CI matrix entry.

#![cfg(target_os = "linux")]

use base64::{Engine as _, engine::general_purpose::STANDARD};
use infra_e2e::{Faults, Fixture};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, time::Duration};

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn decode_chunk(body: &Value) -> Vec<u8> {
    STANDARD
        .decode(body["data_base64"].as_str().expect("chunk data"))
        .expect("decode chunk")
}

async fn terminal_read(fixture: &Fixture, session: &str, offset: u64, length: usize) -> Value {
    let result = fixture
        .mcp
        .call(
            "terminal",
            json!({
                "device": fixture.device_id,
                "action": "read",
                "session_id": session,
                "offset": offset,
                "length": length,
            }),
        )
        .await
        .expect("terminal read");
    assert_eq!(result["status"], "ok", "terminal read failed: {result}");
    result["body"].clone()
}

/// Read from `offset` until `marker` has arrived and the retained range stops
/// growing, then return the accumulated bytes and the next offset.
async fn read_until_stable(
    fixture: &Fixture,
    session: &str,
    offset: u64,
    marker: &str,
) -> (String, u64) {
    let mut output = String::new();
    let mut next = offset;
    let mut quiet = 0;
    for _ in 0..300 {
        let body = terminal_read(fixture, session, next, 64 * 1024).await;
        let chunk = body["output"].as_str().unwrap_or_default().to_owned();
        let chunk_next = body["next_offset"].as_u64().expect("next offset");
        assert_eq!(
            body["offset"].as_u64(),
            Some(next),
            "terminal read did not start at the requested offset: {body}"
        );
        if chunk.is_empty() && chunk_next == next {
            quiet += 1;
        } else {
            quiet = 0;
        }
        output.push_str(&chunk);
        next = chunk_next;
        if output.contains(marker) && quiet >= 3 {
            return (output, next);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("terminal output never contained {marker:?}: {output:?}");
}

/// Scenario 1: a mutating command is dispatched, the socket dies before the
/// response returns, and the gateway must report `outcome_unknown` without ever
/// replaying the command after the reconnect.
#[tokio::test]
async fn exec_cut_before_response_is_outcome_unknown_and_never_replayed() {
    let fixture = Fixture::start().await;
    let marker = fixture.dir.path().join("exec-marker.txt");
    let started = fixture.dir.path().join("exec-started.txt");
    let command = format!(
        "printf s >> {}; sleep 1; printf x >> {}",
        started.display(),
        marker.display()
    );
    let connection = fixture.proxy.current_connection();
    let mcp = fixture.mcp.clone();
    let device = fixture.device_id.clone();
    let call = tokio::spawn(async move {
        mcp.call("exec", json!({"device": device, "command": command}))
            .await
    });
    // Cut only once the command is provably running, so the response is in
    // flight rather than the request still being queued.
    for _ in 0..100 {
        if started.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(started.exists(), "the exec command never started");
    fixture.proxy.cut(connection);
    let result = call.await.expect("exec task").expect("exec call");
    assert_eq!(
        result["status"], "outcome_unknown",
        "a lost response must be ambiguous, not a failure or a success: {result}"
    );
    assert!(
        result["request_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "outcome_unknown must carry the stable request id: {result}"
    );

    // The command still ran exactly once on the device.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        fs::read_to_string(&marker).expect("exec side effect"),
        "x",
        "the dispatched command must have run exactly once"
    );

    // The agent reconnects and nothing is replayed.
    fixture.wait_online().await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        fs::read_to_string(&marker).expect("exec side effect"),
        "x",
        "the command must not be replayed after reconnect"
    );
    let rows = fixture.store.audit_rows();
    let exec_rows: Vec<_> = rows.iter().filter(|row| row.action == "exec").collect();
    assert_eq!(exec_rows.len(), 1, "exactly one exec audit row: {rows:?}");
    assert_eq!(exec_rows[0].outcome, "outcome_unknown");
    assert_eq!(
        exec_rows[0].request_id.as_deref(),
        result["request_id"].as_str()
    );
}

/// Scenario 2: a download is cut mid-chunk and resumes from the last known
/// offset with an identical digest.
#[tokio::test]
async fn download_resumes_from_offset_after_a_mid_chunk_cut() {
    let fixture = Fixture::start().await;
    let source = fixture.dir.path().join("download-source.bin");
    let data: Vec<u8> = (0..1024 * 1024).map(|index| (index % 251) as u8).collect();
    fs::write(&source, &data).expect("write download source");
    let expected = sha256_hex(&data);

    // The real agent advertises transfer.binary_v1, so this scenario exercises
    // the bulk-frame download path; the gateway unit test pins the selection.
    let device = fixture.device().await;
    assert!(
        device["capabilities"]
            .as_array()
            .expect("capabilities")
            .iter()
            .any(|capability| capability == "transfer.binary_v1"),
        "the agent must advertise transfer.binary_v1: {device}"
    );

    let begin = fixture
        .mcp
        .call(
            "transfer",
            json!({
                "device": fixture.device_id,
                "action": "download_begin",
                "path": source,
            }),
        )
        .await
        .expect("download begin");
    assert_eq!(begin["status"], "ok", "download begin failed: {begin}");
    let transfer = begin["body"]["transfer"].clone();
    let transfer_id = transfer["id"].as_str().expect("transfer id").to_owned();
    let size = transfer["size"].as_u64().expect("transfer size");
    assert_eq!(size, data.len() as u64);
    assert_eq!(transfer["sha256"], expected);

    // Deliver the first chunk through tiny TCP writes so frame reassembly is
    // exercised on a real socket, not just in-process.
    let connection = fixture.proxy.current_connection();
    fixture.proxy.set_faults(
        connection,
        Faults {
            split_bytes: 97,
            ..Faults::default()
        },
    );
    let first = fixture
        .mcp
        .call(
            "transfer",
            json!({
                "device": fixture.device_id,
                "action": "download_chunk",
                "transfer_id": transfer_id,
                "offset": 0,
                "length": 262_144,
            }),
        )
        .await
        .expect("first chunk");
    assert_eq!(first["status"], "ok", "first chunk failed: {first}");
    let mut received = decode_chunk(&first["body"]);
    assert_eq!(received.len(), 262_144);
    assert_eq!(first["body"]["next_offset"].as_u64(), Some(262_144));

    // Cut while the next chunk is in flight.
    fixture.proxy.set_faults(
        connection,
        Faults {
            delay_downstream_ms: 800,
            ..Faults::default()
        },
    );
    let mcp = fixture.mcp.clone();
    let device = fixture.device_id.clone();
    let transfer_for_call = transfer_id.clone();
    let call = tokio::spawn(async move {
        mcp.call(
            "transfer",
            json!({
                "device": device,
                "action": "download_chunk",
                "transfer_id": transfer_for_call,
                "offset": 262_144,
                "length": 262_144,
            }),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    fixture.proxy.cut(connection);
    let lost = call.await.expect("chunk task").expect("chunk call");
    assert_eq!(
        lost["status"], "outcome_unknown",
        "a chunk lost in flight must be ambiguous: {lost}"
    );

    // Resume from the last offset that was actually received.
    fixture.wait_online().await;
    let mut offset = received.len() as u64;
    while offset < size {
        let chunk = fixture
            .mcp
            .call(
                "transfer",
                json!({
                    "device": fixture.device_id,
                    "action": "download_chunk",
                    "transfer_id": transfer_id,
                    "offset": offset,
                    "length": 262_144,
                }),
            )
            .await
            .expect("resumed chunk");
        assert_eq!(chunk["status"], "ok", "resumed chunk failed: {chunk}");
        assert_eq!(
            chunk["body"]["offset"].as_u64(),
            Some(offset),
            "the agent must serve the requested offset: {chunk}"
        );
        let bytes = decode_chunk(&chunk["body"]);
        assert!(!bytes.is_empty(), "resume made no progress: {chunk}");
        received.extend_from_slice(&bytes);
        offset = chunk["body"]["next_offset"].as_u64().expect("next offset");
    }
    assert_eq!(received.len() as u64, size);
    assert_eq!(
        sha256_hex(&received),
        expected,
        "resumed download must be byte-identical to the source"
    );

    let status = fixture
        .mcp
        .call(
            "transfer",
            json!({
                "device": fixture.device_id,
                "action": "status",
                "transfer_id": transfer_id,
            }),
        )
        .await
        .expect("transfer status");
    assert_eq!(status["body"]["transfer"]["status"], "completed");
    assert_eq!(status["body"]["transfer"]["sha256"], expected);
}

/// Scenario 3: cross-device copy cut mid-stream. The durable copy queue does not
/// exist yet, so this stays an explicit placeholder rather than a silent gap.
#[tokio::test]
#[ignore = "TODO(W2): needs the durable cross-device copy queue"]
async fn copy_cut_mid_stream_resumes_after_gateway_restart() {
    // TODO(W2): once copy_jobs exists, start a copy, kill the gateway mid-stream,
    // restart it, and assert the job resumes at the stored offset with a matching
    // digest and no duplicated side effects.
    panic!("W2 durable copy queue is not implemented");
}

/// Scenario 4: a terminal survives a cut and the retained byte range has no lost
/// or duplicated bytes.
#[tokio::test]
async fn terminal_output_survives_a_cut_without_lost_or_duplicated_bytes() {
    let fixture = Fixture::start().await;
    let open = fixture
        .mcp
        .call(
            "terminal",
            json!({
                "device": fixture.device_id,
                "action": "open",
                "command": "sh",
                "rows": 24,
                "cols": 80,
            }),
        )
        .await
        .expect("terminal open");
    assert_eq!(open["status"], "ok", "terminal open failed: {open}");
    let session = open["body"]["session"]["id"]
        .as_str()
        .expect("session id")
        .to_owned();

    // The marker is assembled by printf so the PTY echo of the command line does
    // not contain it; a single occurrence proves the output was not duplicated.
    fixture
        .mcp
        .call(
            "terminal",
            json!({
                "device": fixture.device_id,
                "action": "write",
                "session_id": session,
                "data": "printf 'MARK-%s\\n' A\n",
            }),
        )
        .await
        .expect("terminal write A");
    let (first, first_next) = read_until_stable(&fixture, &session, 0, "MARK-A").await;

    fixture
        .mcp
        .call(
            "terminal",
            json!({
                "device": fixture.device_id,
                "action": "write",
                "session_id": session,
                "data": "printf 'MARK-%s\\n' B\n",
            }),
        )
        .await
        .expect("terminal write B");

    let connection = fixture.proxy.current_connection();
    fixture.proxy.cut(connection);
    fixture.wait_online().await;

    let (second, second_next) = read_until_stable(&fixture, &session, first_next, "MARK-B").await;
    let full = terminal_read(&fixture, &session, 0, 64 * 1024).await;
    assert_eq!(
        full["next_offset"].as_u64(),
        Some(second_next),
        "the retained range must end where the incremental reads ended: {full}"
    );
    let full_output = full["output"].as_str().expect("full output");
    assert_eq!(
        full_output,
        format!("{first}{second}"),
        "incremental reads must equal the retained range byte for byte"
    );
    assert_eq!(
        full_output.matches("MARK-A").count(),
        1,
        "no duplicated bytes: {full_output:?}"
    );
    assert_eq!(
        full_output.matches("MARK-B").count(),
        1,
        "no lost bytes: {full_output:?}"
    );

    // The session is still the same durable PTY after the reconnect.
    let list = fixture
        .mcp
        .call(
            "terminal",
            json!({"device": fixture.device_id, "action": "list"}),
        )
        .await
        .expect("terminal list");
    let sessions = list["body"]["sessions"].as_array().expect("sessions");
    assert!(
        sessions
            .iter()
            .any(|entry| entry["id"].as_str() == Some(session.as_str())),
        "the PTY must survive the reconnect: {list}"
    );

    // Durable PTY workers outlive the controller, so the scenario closes its own
    // session instead of leaving an orphan process behind.
    let closed = fixture
        .mcp
        .call(
            "terminal",
            json!({
                "device": fixture.device_id,
                "action": "close",
                "session_id": session,
            }),
        )
        .await
        .expect("terminal close");
    assert_eq!(closed["status"], "ok", "terminal close failed: {closed}");
}

/// Scenario 6: a second connection supersedes the first, and the old socket
/// closing must not flip the device offline.
#[tokio::test]
async fn a_second_connection_wins_and_the_old_socket_close_does_not_go_offline() {
    let mut fixture = Fixture::start().await;
    let first_generation = fixture.generation().await;
    let mut second = fixture.spawn_second_agent().expect("second agent");
    let second_generation = fixture.wait_for_generation_above(first_generation).await;
    assert!(second_generation > first_generation);

    // The superseded socket closes after the new generation is live.
    fixture.agent.stop();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let device = fixture.device().await;
    assert_eq!(
        device["online"].as_bool(),
        Some(true),
        "the old socket closing must not flip the device offline: {device}"
    );
    assert_eq!(
        device["connection_generation"].as_u64(),
        Some(second_generation)
    );
    assert_eq!(
        fixture.store.audit_count("device.disconnect"),
        0,
        "a superseded socket is not a disconnect: {:?}",
        fixture.store.audit_rows()
    );
    assert_eq!(
        fixture.store.audit_count("device.recovered"),
        0,
        "a supersession is not a recovery: {:?}",
        fixture.store.audit_rows()
    );
    // The superseded connection must be classified as a supersession, not as a
    // failed control-path health check: the soak analysis reads these lines.
    let log = fixture.gateway.log();
    assert!(
        log.contains("superseded by a newer connection"),
        "the supersession must be logged as such"
    );
    assert!(
        !log.contains("failed control-path health check"),
        "a supersession must not be reported as a health failure"
    );
    second.stop();
}

/// Scenario 7: a durable job survives a gateway restart and the agent reconciles
/// it on the new connection.
#[tokio::test]
async fn a_durable_job_survives_a_gateway_restart_and_the_agent_reconciles() {
    let mut fixture = Fixture::start().await;
    let start = fixture
        .mcp
        .call(
            "jobs",
            json!({
                "device": fixture.device_id,
                "action": "start",
                "command": "sleep 6; printf job-done",
            }),
        )
        .await
        .expect("job start");
    assert_eq!(start["status"], "ok", "job start failed: {start}");
    let job_id = start["body"]["job"]["id"]
        .as_str()
        .expect("job id")
        .to_owned();
    fixture.wait_active_jobs(1).await;

    fixture.gateway.restart().await.expect("restart gateway");
    fixture.wait_online().await;
    // The agent's first reconcile on the new connection carries the running job.
    fixture.wait_active_jobs(1).await;

    let mut output = String::new();
    let mut status = String::new();
    for _ in 0..200 {
        let read = fixture
            .mcp
            .call(
                "jobs",
                json!({
                    "device": fixture.device_id,
                    "action": "read",
                    "job_id": job_id,
                }),
            )
            .await
            .expect("job read");
        assert_eq!(read["status"], "ok", "job read failed: {read}");
        status = read["body"]["job"]["status"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        output = read["body"]["output"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if status == "completed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(status, "completed", "job did not finish: {output:?}");
    assert!(
        output.contains("job-done"),
        "job output was lost across the restart: {output:?}"
    );
    // A killed gateway cannot record a disconnect, so the reconnect is a plain
    // connect: two connect rows, no disconnect row and no recovery event. The
    // recovery event needs a recorded disconnect (W0.5) and is covered by the
    // heartbeat scenario instead.
    assert_eq!(
        fixture.store.audit_count("device.connect"),
        2,
        "the agent must reconnect exactly once: {:?}",
        fixture.store.audit_rows()
    );
    assert_eq!(fixture.store.audit_count("device.disconnect"), 0);
    let row = fixture
        .store
        .device_row(&fixture.device_id)
        .expect("device row");
    assert_eq!(row.last_disconnect_reason, None);
}

/// Scenario 5: a stalled agent is retired on the heartbeat deadline, recorded as
/// a heartbeat disconnect, and the replacement connection is never clobbered by
/// the retirement.
///
/// Supersession retires the old task immediately (replacing the device entry
/// drops its control channel), so the deadline path is exercised without a
/// replacement: the gateway must notice the silence, close the stale generation,
/// record why, and accept the reconnect that follows.
#[tokio::test]
async fn a_stalled_agent_is_retired_on_the_heartbeat_deadline_and_replaced_cleanly() {
    let fixture = Fixture::start().await;
    let first_generation = fixture.generation().await;
    let connection = fixture.proxy.current_connection();
    // The agent stops sending anything. The gateway treats any inbound frame as
    // liveness, so the whole upstream direction is dropped; dropping only pongs
    // would leave the opportunistic reconcile keeping the link alive.
    fixture.proxy.set_faults(
        connection,
        Faults {
            blackhole_upstream: true,
            ..Faults::default()
        },
    );

    fixture
        .gateway
        .wait_for_log("agent heartbeat timed out", Duration::from_secs(70))
        .await
        .expect("the stalled generation must hit the heartbeat deadline");
    // The warn line is written before the cleanup runs; "agent offline" is
    // logged after the disconnect row is recorded.
    fixture
        .gateway
        .wait_for_log("agent offline", Duration::from_secs(10))
        .await
        .expect("the stale generation must finish its cleanup");
    let row = fixture
        .store
        .device_row(&fixture.device_id)
        .expect("device row");
    assert_eq!(
        row.last_disconnect_reason.as_deref(),
        Some("heartbeat_timeout"),
        "the retirement reason must be recorded: {row:?}"
    );
    assert_eq!(
        fixture.store.audit_count("device.disconnect"),
        1,
        "the stale generation must be recorded as a disconnect: {:?}",
        fixture.store.audit_rows()
    );

    // The gateway closed the stale socket, so the agent reconnects on a newer
    // generation and the retirement must not clobber it.
    let second_generation = fixture.wait_for_generation_above(first_generation).await;
    assert!(second_generation > first_generation);
    assert!(
        fixture.store.audit_count("device.recovered") >= 1,
        "a reconnect after a recorded disconnect must record recovery evidence: {:?}",
        fixture.store.audit_rows()
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    let device = fixture.device().await;
    assert_eq!(
        device["online"].as_bool(),
        Some(true),
        "the replacement connection must stay online: {device}"
    );
    assert_eq!(
        device["connection_generation"].as_u64(),
        Some(second_generation),
        "the retirement must not replace the newer generation: {device}"
    );
}

/// W5: one reconnect produces exactly one changed reconcile, and the idle
/// pings that follow do not drive dashboard churn.
#[tokio::test]
async fn one_reconnect_produces_exactly_one_changed_reconcile() {
    let fixture = Fixture::start().await;
    // Wait for the initial connect's reconcile, then count from a clean slate.
    for _ in 0..100 {
        if fixture
            .events
            .count_for("sessions_changed", &fixture.device_id)
            >= 1
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    fixture.events.drain();

    let connection = fixture.proxy.current_connection();
    fixture.proxy.cut(connection);
    fixture.wait_online().await;
    // Cover at least one heartbeat interval: the opportunistic reconcile after a
    // probe would have produced extra events under the old behaviour.
    tokio::time::sleep(Duration::from_secs(12)).await;

    let changed = fixture
        .events
        .count_for("sessions_changed", &fixture.device_id);
    assert_eq!(
        changed,
        1,
        "one reconnect must produce exactly one changed reconcile: {:?}",
        fixture.events.take()
    );
}

/// Bytes already written into an in-progress upload staging file. The agent
/// creates `.infra-upload-<id>.part` next to the destination and appends each
/// chunk before it answers, so a non-zero size proves the copy is mid-stream.
fn staging_bytes(dir: &std::path::Path) -> u64 {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".infra-upload-")
        })
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .sum()
}

/// W1: a cross-device copy over the bulk-frame path is cut while a chunk is in
/// flight and resumes from the last confirmed offset with an identical digest.
#[tokio::test]
async fn copy_between_two_devices_resumes_after_a_cut_on_the_binary_path() {
    let fixture = Fixture::start().await;
    let source_connection = fixture.proxy.current_connection();
    let (mut destination_agent, destination_id) = fixture
        .spawn_second_device("e2e-destination")
        .await
        .expect("second device");
    let destination_connection = fixture.proxy.current_connection();
    assert_ne!(
        source_connection, destination_connection,
        "the second device must open its own proxied connection"
    );
    let source = fixture.dir.path().join("copy-source.bin");
    let destination_path = fixture.dir.path().join("copy-destination.bin");
    let data: Vec<u8> = (0..1024 * 1024).map(|index| (index % 251) as u8).collect();
    fs::write(&source, &data).expect("write copy source");
    let expected = sha256_hex(&data);

    // Both agents advertise the capability, so the relay must use bulk frames.
    let devices = fixture.devices().await;
    for device_id in [&fixture.device_id, &destination_id] {
        let device = devices
            .iter()
            .find(|device| device["device_id"] == json!(device_id))
            .expect("device listing");
        assert!(
            device["capabilities"]
                .as_array()
                .expect("capabilities")
                .iter()
                .any(|capability| capability == "transfer.binary_v1"),
            "the agent must advertise transfer.binary_v1: {device}"
        );
    }

    let destination_generation = fixture.device_generation(&destination_id).await;
    // Slow the destination link so the cut lands while a chunk is in flight. The
    // proxy delays each forwarded read, so a 256 KiB frame takes several
    // hundred milliseconds instead of arriving in one burst.
    fixture.proxy.set_faults(
        destination_connection,
        Faults {
            delay_downstream_ms: 200,
            ..Faults::default()
        },
    );
    let mcp = fixture.mcp.clone();
    let (from, to) = (fixture.device_id.clone(), destination_id.clone());
    let (source_for_call, destination_for_call) = (source.clone(), destination_path.clone());
    let call = tokio::spawn(async move {
        mcp.call(
            "copy",
            json!({
                "source_device": from,
                "destination_device": to,
                "source_path": source_for_call,
                "destination_path": destination_for_call,
                "max_bytes": 4 * 262_144,
            }),
        )
        .await
    });
    let mut progressed = false;
    for _ in 0..750 {
        if staging_bytes(fixture.dir.path()) > 0 {
            progressed = true;
            break;
        }
        if call.is_finished() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        progressed,
        "the copy never started writing the destination before it finished"
    );
    fixture.proxy.cut(destination_connection);
    let first = call.await.expect("copy task").expect("copy call");
    assert_ne!(
        first["status"], "completed",
        "the cut must interrupt the copy: {first}"
    );
    let source_transfer = first["source_transfer_id"]
        .as_str()
        .expect("source transfer id")
        .to_owned();
    let destination_transfer = first["destination_transfer_id"]
        .as_str()
        .expect("destination transfer id")
        .to_owned();
    let confirmed = first["bytes_transferred"]
        .as_u64()
        .expect("confirmed offset");
    assert!(
        confirmed < data.len() as u64,
        "the copy must be partial: {first}"
    );

    // The cut link reconnects; the resume continues from the confirmed offset.
    fixture
        .wait_device_generation_above(&destination_id, destination_generation)
        .await;

    let mut result = first.clone();
    for _ in 0..40 {
        result = fixture
            .mcp
            .call(
                "copy",
                json!({
                    "source_device": fixture.device_id,
                    "destination_device": destination_id,
                    "source_path": source,
                    "destination_path": destination_path,
                    "source_transfer_id": source_transfer,
                    "destination_transfer_id": destination_transfer,
                    "max_bytes": 64 * 1024 * 1024,
                }),
            )
            .await
            .expect("resume copy");
        if result["status"] == "completed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(result["status"], "completed", "resume failed: {result}");
    assert_eq!(
        result["transport"]["source"], "binary",
        "the source must use bulk frames: {result}"
    );
    assert_eq!(
        result["transport"]["destination"], "binary",
        "the destination must use bulk frames: {result}"
    );
    assert_eq!(result["sha256"], expected);
    assert_eq!(
        result["bytes_transferred"].as_u64(),
        Some(data.len() as u64)
    );
    assert_eq!(
        sha256_hex(&fs::read(&destination_path).expect("destination file")),
        expected,
        "the resumed copy must be byte-identical to the source"
    );
    destination_agent.stop();
}
