use super::*;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use std::collections::BTreeMap;
use tempfile::TempDir;
use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};
const TEST_TOKEN: &str = "developmenttoken123456";

fn test_device(id: &str, name: &str) -> DeviceRecord {
    DeviceRecord {
        device_id: id.to_owned(),
        device_name: name.to_owned(),
        display_name: None,
        public_key: None,
        platform: "linux".into(),
        arch: "aarch64".into(),
        agent_version: "test".into(),
        boot_id: "boot-test".into(),
        capabilities: vec!["exec".into()],
        metadata: serde_json::json!({}),
        paired_at_unix_ms: 1,
        last_seen_unix_ms: 1,
        revoked: false,
        reconcile: None,
        last_disconnect_reason: None,
        last_disconnect_at_unix_ms: None,
    }
}

#[test]
fn malformed_exec_payload_is_a_failure_not_a_successful_command() {
    let output = exec_output(AgentResponse {
        id: Uuid::new_v4(),
        result: OperationResult::Ok {
            body: serde_json::json!({"unexpected":"shape"}),
        },
    });
    assert_eq!(output.status, "failed");
    assert!(output.error.unwrap().contains("invalid exec response"));
}

#[test]
fn friendly_names_and_ids_resolve_without_confusing_offline_and_unknown() {
    let records = vec![
        test_device("vm-id", "oracle-vm"),
        test_device("mac-id", "BigMac"),
    ];
    assert_eq!(
        select_device(&records, "oracle-vm").unwrap().device_id,
        "vm-id"
    );
    assert_eq!(
        select_device(&records, "BIGMAC").unwrap().device_id,
        "mac-id"
    );
    assert_eq!(select_device(&records, "vm-id").unwrap().device_id, "vm-id");
    assert!(matches!(
        select_device(&records, "unknown"),
        Err(DispatchError::NotFound(_))
    ));
    assert!(matches!(
        select_device(&records, " "),
        Err(DispatchError::NotFound(_))
    ));
}

#[test]
fn duplicate_names_require_an_unambiguous_device_id() {
    let records = vec![test_device("a-id", "Laptop"), test_device("b-id", "Laptop")];
    assert!(matches!(
        select_device(&records, "Laptop"),
        Err(DispatchError::Ambiguous { .. })
    ));
    assert_eq!(select_device(&records, "b-id").unwrap().device_id, "b-id");
}

#[test]
fn revoked_devices_do_not_route() {
    let mut record = test_device("bad-id", "RevokedMac");
    record.revoked = true;
    assert!(matches!(
        select_device(&[record], "RevokedMac"),
        Err(DispatchError::Revoked(_))
    ));
}
async fn spawn_test_gateway() -> (TempDir, AppState, SocketAddr, tokio::task::JoinHandle<()>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("infra.db")).unwrap();
    let state = AppState::new(store, Some(TEST_TOKEN.into()), None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = build_app(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (dir, state, addr, task)
}
fn test_hello(device_id: &str, instance: Uuid) -> DeviceHello {
    DeviceHello {
        protocol: PROTOCOL_VERSION,
        device_id: device_id.into(),
        device_name: device_id.into(),
        agent_version: "test".into(),
        platform: "linux".into(),
        arch: "aarch64".into(),
        boot_id: "boot-test".into(),
        agent_instance_id: instance,
        capabilities: vec!["exec".into()],
        metadata: BTreeMap::new(),
    }
}

type TestSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn next_test_message(socket: &mut TestSocket) -> GatewayToAgent {
    let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ClientMessage::Text(text) = frame else {
        panic!("expected JSON frame")
    };
    serde_json::from_str(text.as_str()).unwrap()
}
fn enroll_readiness_fixture(state: &AppState) -> (String, SigningKey) {
    let key = SigningKey::from_bytes(&[9; 32]);
    let enrollment = state.store.create_enrollment(60_000).unwrap();
    let enrolled = state
        .store
        .consume_enrollment(
            &enrollment.code,
            &Uuid::new_v4().to_string(),
            "readiness-fixture",
            &key.verifying_key().to_bytes(),
            "linux",
            "aarch64",
            "test",
        )
        .unwrap();
    (enrolled.device_id, key)
}
async fn connect_readiness_fixture(
    addr: SocketAddr,
    id: &str,
    key: &SigningKey,
    advertise: bool,
) -> (TestSocket, HelloAck) {
    let (mut socket, _) = connect_async(format!("ws://{addr}/agent")).await.unwrap();
    let mut hello = test_hello(id, Uuid::new_v4());
    if advertise {
        hello
            .capabilities
            .push(infra_protocol::CONNECTION_READINESS_CAPABILITY.into());
    }
    socket
        .send(ClientMessage::Text(
            serde_json::to_string(&AgentToGateway::AuthStart(DeviceAuthStart {
                protocol: PROTOCOL_VERSION,
                device_id: id.into(),
                agent_instance_id: hello.agent_instance_id,
            }))
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let GatewayToAgent::AuthChallenge(challenge) = next_test_message(&mut socket).await else {
        panic!("expected challenge")
    };
    let signature = URL_SAFE_NO_PAD.encode(
        key.sign(&signed_auth_message(
            &challenge.nonce,
            challenge.gateway_epoch,
            &hello,
        ))
        .to_bytes(),
    );
    socket
        .send(ClientMessage::Text(
            serde_json::to_string(&AgentToGateway::SignedHello(SignedDeviceHello {
                hello,
                signature,
            }))
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let GatewayToAgent::HelloAck(ack) = next_test_message(&mut socket).await else {
        panic!("expected ack")
    };
    (socket, ack)
}

#[tokio::test]
async fn signed_readiness_status_roundtrip_rejects_nonce_replay_and_preserves_legacy_peers() {
    let (_dir, state, addr, server) = spawn_test_gateway().await;
    let (id, key) = enroll_readiness_fixture(&state);
    let (mut socket, ack) = connect_readiness_fixture(addr, &id, &key, true).await;
    let GatewayToAgent::ConnectionProbe(probe) = next_test_message(&mut socket).await else {
        panic!("expected probe")
    };
    assert_eq!(probe.generation, ack.connection_generation);
    let reply = infra_protocol::ConnectionProbeReply {
        probe,
        pid: 42,
        protocol: PROTOCOL_VERSION,
    };
    let mut wrong = reply.clone();
    wrong.probe.nonce = Uuid::new_v4();
    socket
        .send(ClientMessage::Text(
            serde_json::to_string(&AgentToGateway::ConnectionProbeReply(wrong))
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), socket.next())
            .await
            .is_err()
    );
    socket
        .send(ClientMessage::Text(
            serde_json::to_string(&AgentToGateway::ConnectionProbeReply(reply.clone()))
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        next_test_message(&mut socket).await,
        GatewayToAgent::ConnectionReady(reply.clone())
    );
    socket
        .send(ClientMessage::Text(
            serde_json::to_string(&AgentToGateway::ConnectionProbeReply(reply))
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), socket.next())
            .await
            .is_err()
    );
    let (mut old, _) = connect_readiness_fixture(addr, &id, &key, false).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), old.next())
            .await
            .is_err(),
        "non-advertising peer must not receive extension frames"
    );
    server.abort();
}

#[tokio::test]
async fn a_superseded_signed_connection_cannot_confirm_readiness() {
    let (_dir, state, addr, server) = spawn_test_gateway().await;
    let (id, key) = enroll_readiness_fixture(&state);
    let (mut old, old_ack) = connect_readiness_fixture(addr, &id, &key, true).await;
    let GatewayToAgent::ConnectionProbe(old_probe) = next_test_message(&mut old).await else {
        panic!()
    };
    let (mut current, ack) = connect_readiness_fixture(addr, &id, &key, true).await;
    let GatewayToAgent::ConnectionProbe(probe) = next_test_message(&mut current).await else {
        panic!()
    };
    assert_ne!(old_ack.connection_generation, ack.connection_generation);
    old.send(ClientMessage::Text(
        serde_json::to_string(&AgentToGateway::ConnectionProbeReply(
            infra_protocol::ConnectionProbeReply {
                probe: old_probe,
                pid: 42,
                protocol: PROTOCOL_VERSION,
            },
        ))
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(1), old.next())
        .await
        .unwrap();
    assert!(
        !matches!(frame, Some(Ok(ClientMessage::Text(_)))),
        "stale socket received a readiness confirmation"
    );
    let reply = infra_protocol::ConnectionProbeReply {
        probe,
        pid: 43,
        protocol: PROTOCOL_VERSION,
    };
    current
        .send(ClientMessage::Text(
            serde_json::to_string(&AgentToGateway::ConnectionProbeReply(reply.clone()))
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        next_test_message(&mut current).await,
        GatewayToAgent::ConnectionReady(reply)
    );
    assert_eq!(
        state.devices.read().await[&id].generation,
        ack.connection_generation
    );
    server.abort();
}
async fn connect_legacy_with_capabilities(
    addr: SocketAddr,
    device_id: &str,
    capabilities: &[&str],
) -> (TestSocket, HelloAck) {
    let (mut socket, _) = connect_async(format!("ws://{addr}/agent")).await.unwrap();
    let mut hello = test_hello(device_id, Uuid::new_v4());
    hello.capabilities = capabilities.iter().map(|cap| (*cap).to_owned()).collect();
    let message = AgentToGateway::LegacyHello(LegacyDeviceHello {
        hello,
        token: TEST_TOKEN.into(),
    });
    socket
        .send(ClientMessage::Text(
            serde_json::to_string(&message).unwrap().into(),
        ))
        .await
        .unwrap();
    let ClientMessage::Text(text) = socket.next().await.unwrap().unwrap() else {
        panic!()
    };
    let GatewayToAgent::HelloAck(ack) = serde_json::from_str(text.as_str()).unwrap() else {
        panic!()
    };
    (socket, ack)
}

async fn connect_legacy(addr: SocketAddr, device_id: &str) -> (TestSocket, u64) {
    let (socket, ack) = connect_legacy_with_capabilities(addr, device_id, &["exec"]).await;
    (socket, ack.connection_generation)
}
async fn wait_registered(state: &AppState, device_id: &str) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if state.devices.read().await.contains_key(device_id) {
                break;
            }
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap()
}
#[test]
fn browser_session_survives_gateway_state_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("first.db")).unwrap();
    let state = AppState::new(store, None, Some(TEST_TOKEN.into()));
    let (cookie, expires) = create_browser_session_cookie(TEST_TOKEN.as_bytes()).unwrap();
    assert!(expires > now_ms());
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        axum::http::HeaderValue::from_str(&format!("{BROWSER_SESSION_COOKIE}={cookie}")).unwrap(),
    );
    assert!(browser_session_expiry(&state, &headers).is_some());

    let restarted = AppState::new(
        Store::open(dir.path().join("second.db")).unwrap(),
        None,
        Some(TEST_TOKEN.into()),
    );
    assert!(browser_session_expiry(&restarted, &headers).is_some());
    assert!(!bearer_authorized(&restarted, &headers));

    let mut tampered = headers.clone();
    tampered.insert(
        header::COOKIE,
        axum::http::HeaderValue::from_static("infra_session=v1.1.invalid.invalid"),
    );
    assert!(browser_session_expiry(&restarted, &tampered).is_none());
}

fn reconcile_fixture(status: &str) -> ReconcileSnapshot {
    ReconcileSnapshot {
        items: vec![infra_protocol::ReconcileItem {
            id: Uuid::from_u128(7),
            kind: infra_protocol::ReconcileKind::Job,
            status: status.into(),
            boot_id: "boot-test".into(),
            started_at_unix_ms: 11,
        }],
    }
}

async fn send_reconcile(socket: &mut TestSocket, snapshot: &ReconcileSnapshot) {
    let message = AgentToGateway::Reconcile(snapshot.clone());
    socket
        .send(ClientMessage::Text(
            serde_json::to_string(&message).unwrap().into(),
        ))
        .await
        .unwrap();
}

async fn send_reconcile_unchanged(socket: &mut TestSocket, hash: &str) {
    let message = AgentToGateway::ReconcileUnchanged {
        hash: hash.to_owned(),
    };
    socket
        .send(ClientMessage::Text(
            serde_json::to_string(&message).unwrap().into(),
        ))
        .await
        .unwrap();
}

/// Wait briefly for the next `sessions_changed`; `None` means none arrived.
async fn next_sessions_changed(events: &mut broadcast::Receiver<DashboardEvent>) -> Option<()> {
    loop {
        match tokio::time::timeout(Duration::from_millis(300), events.recv()).await {
            Ok(Ok(event)) if event.kind == "sessions_changed" => return Some(()),
            Ok(Ok(_)) => continue,
            _ => return None,
        }
    }
}

#[tokio::test]
async fn reconcile_hash_suppresses_unchanged_sessions_changed_events() {
    let (_dir, state, addr, server) = spawn_test_gateway().await;
    let (mut socket, ack) = connect_legacy_with_capabilities(
        addr,
        "hash-device",
        &["exec", infra_protocol::RECONCILE_HASH_CAPABILITY],
    )
    .await;
    assert!(
        ack.reconcile_hash,
        "a capable agent must be told the gateway understands the compact frame"
    );
    wait_registered(&state, "hash-device").await;
    let mut events = state.events.subscribe();

    let snapshot = reconcile_fixture("running");
    send_reconcile(&mut socket, &snapshot).await;
    assert!(
        next_sessions_changed(&mut events).await.is_some(),
        "the first snapshot is a change"
    );

    // Identical content, full frame: no dashboard churn.
    send_reconcile(&mut socket, &snapshot).await;
    assert!(
        next_sessions_changed(&mut events).await.is_none(),
        "an identical snapshot must not emit sessions_changed"
    );

    // Compact unchanged frame with the stored hash: no dashboard churn.
    send_reconcile_unchanged(&mut socket, &snapshot.content_hash()).await;
    assert!(
        next_sessions_changed(&mut events).await.is_none(),
        "an unchanged reconcile must not emit sessions_changed"
    );

    // Real content change: exactly one event.
    let changed = reconcile_fixture("completed");
    send_reconcile(&mut socket, &changed).await;
    assert!(
        next_sessions_changed(&mut events).await.is_some(),
        "a changed snapshot must emit sessions_changed"
    );
    let record = state.store.get_device("hash-device").unwrap().unwrap();
    assert_eq!(record.reconcile.unwrap().items[0].status, "completed");

    // A hash the gateway does not recognise refreshes the dashboard from the
    // stored snapshot instead of silently diverging.
    send_reconcile_unchanged(&mut socket, "not-the-stored-hash").await;
    assert!(
        next_sessions_changed(&mut events).await.is_some(),
        "a mismatched unchanged frame must refresh the dashboard"
    );
    server.abort();
}

#[tokio::test]
async fn old_agents_keep_emitting_on_every_reconcile() {
    let (_dir, state, addr, server) = spawn_test_gateway().await;
    let (mut socket, ack) = connect_legacy_with_capabilities(addr, "old-device", &["exec"]).await;
    assert!(
        !ack.reconcile_hash,
        "an old agent must not be told about a frame it cannot send"
    );
    wait_registered(&state, "old-device").await;
    let mut events = state.events.subscribe();
    let snapshot = reconcile_fixture("running");
    for _ in 0..2 {
        send_reconcile(&mut socket, &snapshot).await;
        assert!(
            next_sessions_changed(&mut events).await.is_some(),
            "an agent without the capability keeps the old always-emit behaviour"
        );
    }
    server.abort();
}

#[tokio::test]
async fn a_reconnect_resets_the_reconcile_hash() {
    let (_dir, state, addr, server) = spawn_test_gateway().await;
    let capabilities = ["exec", infra_protocol::RECONCILE_HASH_CAPABILITY];
    let (mut socket, _) =
        connect_legacy_with_capabilities(addr, "reset-device", &capabilities).await;
    wait_registered(&state, "reset-device").await;
    let mut events = state.events.subscribe();
    let snapshot = reconcile_fixture("running");
    send_reconcile(&mut socket, &snapshot).await;
    assert!(next_sessions_changed(&mut events).await.is_some());
    send_reconcile(&mut socket, &snapshot).await;
    assert!(next_sessions_changed(&mut events).await.is_none());

    drop(socket);
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.devices.read().await.contains_key("reset-device") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    // The same content on a new connection is a change again: the dashboard may
    // have missed events while the device was away.
    let (mut socket, _) =
        connect_legacy_with_capabilities(addr, "reset-device", &capabilities).await;
    wait_registered(&state, "reset-device").await;
    let mut events = state.events.subscribe();
    send_reconcile(&mut socket, &snapshot).await;
    assert!(
        next_sessions_changed(&mut events).await.is_some(),
        "the first reconcile after a reconnect must emit"
    );
    server.abort();
}

#[tokio::test]
async fn legacy_handshake_and_exec_round_trip() {
    let (_dir, state, addr, server) = spawn_test_gateway().await;
    let (socket, generation) = connect_legacy(addr, "test-device").await;
    wait_registered(&state, "test-device").await;
    assert_eq!(generation, 1);
    let (mut sink, mut stream) = socket.split();
    let responder = tokio::spawn(async move {
        while let Some(Ok(ClientMessage::Text(text))) = stream.next().await {
            let message: GatewayToAgent = serde_json::from_str(text.as_str()).unwrap();
            if let GatewayToAgent::Request(request) = message {
                let response = AgentToGateway::Response(AgentResponse {
                    id: request.id,
                    result: OperationResult::Ok {
                        body: serde_json::to_value(ExecOutput {
                            exit_code: Some(0),
                            stdout: "integration-ok\n".into(),
                            stderr: String::new(),
                            truncated: false,
                        })
                        .unwrap(),
                    },
                });
                sink.send(ClientMessage::Text(
                    serde_json::to_string(&response).unwrap().into(),
                ))
                .await
                .unwrap();
            }
        }
    });
    let response = dispatch_to_device(
        &state,
        "test-device".into(),
        AgentOperation::Exec {
            command: "ignored".into(),
            cwd: None,
            timeout_ms: Some(2000),
        },
        3000,
    )
    .await
    .unwrap();
    assert!(matches!(response.result, OperationResult::Ok { .. }));
    responder.abort();
    server.abort();
}
#[tokio::test]
async fn timed_out_operation_does_not_disconnect_healthy_agent() {
    let (_dir, state, addr, server) = spawn_test_gateway().await;
    let (socket, generation) = connect_legacy(addr, "slow-device").await;
    wait_registered(&state, "slow-device").await;
    let (mut sink, mut stream) = socket.split();
    let responder = tokio::spawn(async move {
        let mut requests = 0;
        while let Some(Ok(ClientMessage::Text(text))) = stream.next().await {
            let message: GatewayToAgent = serde_json::from_str(text.as_str()).unwrap();
            if let GatewayToAgent::Request(request) = message {
                requests += 1;
                if requests == 1 {
                    // One command never responds, but the agent can process
                    // subsequent requests on its still-healthy connection.
                    continue;
                }
                let answer = AgentToGateway::Response(AgentResponse {
                    id: request.id,
                    result: OperationResult::Ok {
                        body: serde_json::json!({"status": "ready"}),
                    },
                });
                sink.send(ClientMessage::Text(
                    serde_json::to_string(&answer).unwrap().into(),
                ))
                .await
                .unwrap();
            }
        }
    });
    let first = dispatch_to_device(&state, "slow-device".into(), AgentOperation::Status, 50)
        .await
        .unwrap();
    assert!(matches!(
        first.result,
        OperationResult::OutcomeUnknown { .. }
    ));
    assert!(state.pending.is_empty());
    assert_eq!(
        state.devices.read().await["slow-device"].generation,
        generation
    );
    let next = dispatch_to_device(&state, "slow-device".into(), AgentOperation::Status, 1000)
        .await
        .unwrap();
    assert!(matches!(next.result, OperationResult::Ok { .. }));
    assert!(state.pending.is_empty());
    assert_eq!(
        state.devices.read().await["slow-device"].generation,
        generation
    );
    responder.abort();
    server.abort();
}

async fn queued_test_device(state: &AppState) -> (mpsc::Receiver<Outbound>, mpsc::Receiver<()>) {
    let (tx, rx) = mpsc::channel(1);
    let (retire, retire_rx) = mpsc::channel(1);
    state.devices.write().await.insert(
        "queued-device".into(),
        DeviceConnection {
            hello: test_hello("queued-device", Uuid::new_v4()),
            generation: 1,
            tx,
            retire,
            bulk: BulkWaiters::default(),
            reconcile: ReconcileSnapshot::default(),
            reconcile_hash: None,
        },
    );
    (rx, retire_rx)
}

#[tokio::test]
async fn list_pagination_checks_live_capability_before_queueing() {
    let (_dir, state, _addr, server) = spawn_test_gateway().await;
    let (mut rx, mut retire) = queued_test_device(&state).await;
    // A previous build advertised pagination, but the current connection
    // does not. Cached discovery is not permission to send new semantics.
    let mut cached = test_hello("queued-device", Uuid::new_v4());
    cached
        .capabilities
        .push(FILE_LIST_PAGINATION_CAPABILITY.into());
    state.store.record_connect(&cached, None).unwrap();
    for (offset, length) in [(Some(0), None), (None, Some(2))] {
        let result = dispatch_to_device(
            &state,
            "queued-device".into(),
            AgentOperation::Files(FileRequest {
                action: FileAction::List {
                    path: ".".into(),
                    offset,
                    length,
                },
            }),
            5000,
        )
        .await;
        let Err(DispatchError::Unsupported(message)) = result else {
            panic!("old connection must reject paging")
        };
        let error = ToolError::from(dispatch_error(DispatchError::Unsupported(message)));
        assert_eq!(error.code, "unsupported");
        assert!(!error.retryable);
        assert!(error.message.contains("files.list_page"));
        assert!(rx.try_recv().is_err());
        assert!(state.pending.is_empty());
    }
    assert!(retire.try_recv().is_err());
    server.abort();
}

#[tokio::test]
async fn legacy_lists_and_advertised_pages_use_the_existing_request_lifecycle() {
    let (_dir, state, _addr, server) = spawn_test_gateway().await;
    let (mut rx, _retire) = queued_test_device(&state).await;
    for paged in [false, true] {
        if paged {
            state
                .devices
                .write()
                .await
                .get_mut("queued-device")
                .unwrap()
                .hello
                .capabilities
                .push(FILE_LIST_PAGINATION_CAPABILITY.into());
        }
        let operation = AgentOperation::Files(FileRequest {
            action: FileAction::List {
                path: "fixture".into(),
                offset: paged.then_some(2),
                length: paged.then_some(3),
            },
        });
        let sent = operation.clone();
        let caller_state = state.clone();
        let caller = tokio::spawn(async move {
            dispatch_to_device(&caller_state, "queued-device".into(), operation, 1000).await
        });
        let Some(Outbound::Json(GatewayToAgent::Request(request))) =
            tokio::time::timeout(Duration::from_secs(1), rx.recv())
                .await
                .unwrap()
        else {
            panic!("expected list request")
        };
        assert_eq!(request.operation, sent);
        state.pending.complete(
            "queued-device",
            1,
            AgentResponse {
                id: request.id,
                result: OperationResult::Ok {
                    body: serde_json::json!({"entries":[],"has_more":false}),
                },
            },
        );
        assert!(matches!(
            caller.await.unwrap().unwrap().result,
            OperationResult::Ok { .. }
        ));
        assert!(state.pending.is_empty());
    }
    server.abort();
}

#[tokio::test]
async fn full_queue_expires_without_dispatch_or_connection_retirement() {
    let (_dir, state, _addr, server) = spawn_test_gateway().await;
    let (mut rx, mut retire) = queued_test_device(&state).await;
    state.devices.read().await["queued-device"]
        .tx
        .send(GatewayToAgent::Ping.into())
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        dispatch_to_device(&state, "queued-device".into(), AgentOperation::Status, 25),
    )
    .await
    .expect("queue admission must be bounded");
    assert!(matches!(result, Err(DispatchError::QueueBusy)));
    assert!(state.pending.is_empty());
    assert!(matches!(
        rx.recv().await,
        Some(Outbound::Json(GatewayToAgent::Ping))
    ));
    assert!(
        rx.try_recv().is_err(),
        "expired operation was unexpectedly queued"
    );
    assert!(
        retire.try_recv().is_err(),
        "a full queue must not retire the connection"
    );
    server.abort();
}

#[tokio::test]
async fn abort_after_dispatch_removes_waiter_and_does_not_replay() {
    let (_dir, state, _addr, server) = spawn_test_gateway().await;
    let (mut rx, mut retire) = queued_test_device(&state).await;
    let caller_state = state.clone();
    let caller = tokio::spawn(async move {
        dispatch_to_device(
            &caller_state,
            "queued-device".into(),
            AgentOperation::Status,
            5_000,
        )
        .await
    });
    let request = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Outbound::Json(GatewayToAgent::Request(request)) = request else {
        panic!("expected request")
    };
    assert!(!state.pending.is_empty());
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(state.pending.is_empty());
    // A late response after cancellation is harmless.
    state.pending.complete(
        "queued-device",
        1,
        AgentResponse {
            id: request.id,
            result: OperationResult::Ok {
                body: serde_json::json!({}),
            },
        },
    );
    assert!(rx.try_recv().is_err());
    assert!(retire.try_recv().is_err());
    server.abort();
}

#[tokio::test]
async fn abort_while_waiting_for_queue_does_not_dispatch() {
    let (_dir, state, _addr, server) = spawn_test_gateway().await;
    let (mut rx, _retire) = queued_test_device(&state).await;
    state.devices.read().await["queued-device"]
        .tx
        .send(GatewayToAgent::Ping.into())
        .await
        .unwrap();
    let caller_state = state.clone();
    let caller = tokio::spawn(async move {
        dispatch_to_device(
            &caller_state,
            "queued-device".into(),
            AgentOperation::Status,
            5_000,
        )
        .await
    });
    tokio::task::yield_now().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(state.pending.is_empty());
    assert!(matches!(
        rx.recv().await,
        Some(Outbound::Json(GatewayToAgent::Ping))
    ));
    assert!(rx.try_recv().is_err());
    server.abort();
}

#[tokio::test]
async fn socket_loss_after_dispatch_returns_unknown_without_waiting_for_deadline() {
    let (_dir, state, addr, server) = spawn_test_gateway().await;
    let (mut socket, _) = connect_legacy(addr, "disconnect-device").await;
    wait_registered(&state, "disconnect-device").await;
    let caller_state = state.clone();
    let caller = tokio::spawn(async move {
        dispatch_to_device(
            &caller_state,
            "disconnect-device".into(),
            AgentOperation::Status,
            30_000,
        )
        .await
    });
    let frame = tokio::time::timeout(Duration::from_secs(1), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(frame, ClientMessage::Text(_)));
    socket.close(None).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), caller)
        .await
        .expect("socket closure should release the pending caller")
        .unwrap()
        .unwrap();
    assert!(matches!(
        result.result,
        OperationResult::OutcomeUnknown { .. }
    ));
    assert!(state.pending.is_empty());
    server.abort();
}

#[tokio::test]
async fn signed_auth_and_revocation() {
    let (_dir, state, addr, server) = spawn_test_gateway().await;
    let enrollment = state.store.create_enrollment(60_000).unwrap();
    let secret = [9_u8; 32];
    let key = SigningKey::from_bytes(&secret);
    let enrolled = state
        .store
        .consume_enrollment(
            &enrollment.code,
            &Uuid::new_v4().to_string(),
            "signed",
            &key.verifying_key().to_bytes(),
            "linux",
            "aarch64",
            "test",
        )
        .unwrap();
    let (mut socket, _) = connect_async(format!("ws://{addr}/agent")).await.unwrap();
    let instance = Uuid::new_v4();
    socket
        .send(ClientMessage::Text(
            serde_json::to_string(&AgentToGateway::AuthStart(DeviceAuthStart {
                protocol: PROTOCOL_VERSION,
                device_id: enrolled.device_id.clone(),
                agent_instance_id: instance,
            }))
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let ClientMessage::Text(text) = socket.next().await.unwrap().unwrap() else {
        panic!()
    };
    let GatewayToAgent::AuthChallenge(challenge) = serde_json::from_str(text.as_str()).unwrap()
    else {
        panic!()
    };
    let hello = test_hello(&enrolled.device_id, instance);
    let signature = URL_SAFE_NO_PAD.encode(
        key.sign(&signed_auth_message(
            &challenge.nonce,
            challenge.gateway_epoch,
            &hello,
        ))
        .to_bytes(),
    );
    socket
        .send(ClientMessage::Text(
            serde_json::to_string(&AgentToGateway::SignedHello(SignedDeviceHello {
                hello,
                signature,
            }))
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let ClientMessage::Text(text) = socket.next().await.unwrap().unwrap() else {
        panic!()
    };
    assert!(matches!(
        serde_json::from_str::<GatewayToAgent>(text.as_str()).unwrap(),
        GatewayToAgent::HelloAck(_)
    ));
    state.store.revoke(&enrolled.device_id).unwrap();
    drop(socket);
    let (mut retry, _) = connect_async(format!("ws://{addr}/agent")).await.unwrap();
    retry
        .send(ClientMessage::Text(
            serde_json::to_string(&AgentToGateway::AuthStart(DeviceAuthStart {
                protocol: PROTOCOL_VERSION,
                device_id: enrolled.device_id,
                agent_instance_id: Uuid::new_v4(),
            }))
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(1), retry.next())
        .await
        .unwrap();
    assert!(matches!(frame, Some(Ok(ClientMessage::Close(_))) | None));
    server.abort();
}
#[tokio::test]
async fn a_reconnect_after_a_recorded_disconnect_writes_a_recovery_event() {
    let (_dir, state, addr, server) = spawn_test_gateway().await;
    let (first, _) = connect_legacy(addr, "recovering-device").await;
    wait_registered(&state, "recovering-device").await;
    drop(first);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !state.devices.read().await.contains_key("recovering-device") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (second, _) = connect_legacy(addr, "recovering-device").await;
    wait_registered(&state, "recovering-device").await;
    let recovered = state
        .store
        .list_activity(20)
        .unwrap()
        .into_iter()
        .find(|record| record.action == "device.recovered")
        .expect("a reconnect after a recorded disconnect must write a recovery event");
    assert_eq!(recovered.device_id.as_deref(), Some("recovering-device"));
    assert!(
        recovered.outcome.ends_with('s'),
        "outcome must carry the elapsed seconds, got {}",
        recovered.outcome
    );
    drop(second);
    server.abort();
}

#[tokio::test]
async fn newer_connection_supersedes_generation() {
    let (_dir, state, addr, server) = spawn_test_gateway().await;
    let (first, first_generation) = connect_legacy(addr, "same-device").await;
    wait_registered(&state, "same-device").await;
    let (second, second_generation) = connect_legacy(addr, "same-device").await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if state
                .devices
                .read()
                .await
                .get("same-device")
                .map(|d| d.generation)
                == Some(second_generation)
            {
                break;
            }
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    assert!(second_generation > first_generation);
    drop(first);
    drop(second);
    server.abort();
}
