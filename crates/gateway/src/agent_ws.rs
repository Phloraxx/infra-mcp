use super::*;
use tracing::{info, warn};

pub(crate) const AGENT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

// Missed-pong deadline. Agents occasionally stall for tens of seconds (laptop
// sleep transitions, timer coalescing, Wi-Fi/Tailscale path hiccups) and then
// resume on their own; a tight deadline retires connections that would have
// recovered, which surfaces as flapping offline devices. Default is
// deliberately generous; override with INFRA_HEARTBEAT_TIMEOUT_SECS.
pub(crate) fn agent_heartbeat_timeout() -> Duration {
    static TIMEOUT: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        std::env::var("INFRA_HEARTBEAT_TIMEOUT_SECS")
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|secs| *secs >= 30)
            .map_or_else(|| Duration::from_secs(90), Duration::from_secs)
    })
}

#[derive(Clone)]
pub(crate) struct DeviceConnection {
    pub(crate) hello: DeviceHello,
    pub(crate) generation: u64,
    pub(crate) tx: mpsc::Sender<GatewayToAgent>,
    pub(crate) retire: mpsc::Sender<()>,
    pub(crate) reconcile: ReconcileSnapshot,
    /// Content hash of `reconcile`, used to suppress redundant
    /// `sessions_changed` events. Reset with the connection, so the first
    /// reconcile after a reconnect always counts as a change.
    pub(crate) reconcile_hash: Option<String>,
}

pub(crate) async fn agent_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.max_message_size(DEFAULT_CONTROL_FRAME_LIMIT)
        .on_upgrade(move |socket| handle_agent(socket, state))
}

pub(crate) async fn handle_agent(mut socket: WebSocket, state: AppState) {
    let auth = match authenticate_agent(&mut socket, &state).await {
        Ok(v) => v,
        Err(e) => {
            warn!(error=%e,"agent authentication rejected");
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    let hello = auth.hello;
    let supports_readiness = auth.public_key.is_some()
        && hello
            .capabilities
            .iter()
            .any(|cap| cap == infra_protocol::CONNECTION_READINESS_CAPABILITY);
    // Only an agent that advertises the capability gets the hash-gated
    // behaviour; an old agent keeps emitting on every reconcile exactly as
    // before.
    let supports_reconcile_hash = hello
        .capabilities
        .iter()
        .any(|cap| cap == infra_protocol::RECONCILE_HASH_CAPABILITY);
    // Bulk frames are only used with an agent that advertised them; an old agent
    // keeps the base64/JSON path and never sees a binary frame.
    let supports_transfer_binary = hello
        .capabilities
        .iter()
        .any(|cap| cap == infra_protocol::TRANSFER_BINARY_CAPABILITY);
    let supports_terminal_binary = hello
        .capabilities
        .iter()
        .any(|cap| cap == infra_protocol::TERMINAL_BINARY_CAPABILITY);
    let generation = state.next_generation.fetch_add(1, Ordering::Relaxed);
    let (out_tx, mut out_rx) = mpsc::channel::<GatewayToAgent>(64);
    let (retire_tx, mut retire_rx) = mpsc::channel::<()>(1);
    let ack = GatewayToAgent::HelloAck(HelloAck {
        protocol: PROTOCOL_VERSION,
        connection_generation: generation,
        gateway_epoch: state.gateway_epoch,
        control_frame_limit: DEFAULT_CONTROL_FRAME_LIMIT,
        reconcile_requested: true,
        reconcile_hash: supports_reconcile_hash,
        binary_frames: supports_transfer_binary || supports_terminal_binary,
    });
    if send_json(&mut socket, &ack).await.is_err() {
        return;
    }
    if let Err(e) = state
        .store
        .record_connect(&hello, auth.public_key.as_deref())
    {
        warn!(device_id=%hello.device_id,error=%e,"failed to persist device connect");
    }
    let was_online = {
        let mut devices = state.devices.write().await;
        devices
            .insert(
                hello.device_id.clone(),
                DeviceConnection {
                    hello: hello.clone(),
                    generation,
                    tx: out_tx.clone(),
                    retire: retire_tx,
                    reconcile: ReconcileSnapshot::default(),
                    reconcile_hash: None,
                },
            )
            .is_some()
    };
    let _ = state
        .store
        .record_activity(Some(&hello.device_id), "device.connect", "ok", None);
    // A supersession is not a recovery: only a device that was absent from the
    // live map has come back from a recorded disconnect.
    if !was_online {
        match state.store.record_recovery(&hello.device_id) {
            Ok(Some(seconds)) => {
                info!(device_id=%hello.device_id,elapsed_secs=seconds,"agent recovered after a recorded disconnect");
            }
            Ok(None) => {}
            Err(e) => {
                warn!(device_id=%hello.device_id,error=%e,"failed to record device recovery");
            }
        }
    }
    state.emit("device_online", Some(hello.device_id.clone()));
    info!(device_id=%hello.device_id,generation,platform=%hello.platform,arch=%hello.arch,"agent online");
    let mut heartbeat = tokio::time::interval(AGENT_HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // `interval` ticks immediately once. Consume that first tick so a newly authenticated
    // connection gets a full interval before its first protocol heartbeat.
    heartbeat.tick().await;
    let mut readiness = readiness::ProbeState::default();
    if supports_readiness {
        let _ = out_tx.try_send(GatewayToAgent::ConnectionProbe(
            readiness.challenge(generation),
        ));
    }
    let mut last_inbound = Instant::now();
    let mut disconnect_reason = "connection_closed";
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let silent_for = last_inbound.elapsed();
                if silent_for >= agent_heartbeat_timeout() {
                    warn!(
                        device_id=%hello.device_id,
                        generation,
                        elapsed_ms = silent_for.as_millis() as u64,
                        deadline_ms = agent_heartbeat_timeout().as_millis() as u64,
                        "agent heartbeat timed out; retiring stale connection"
                    );
                    disconnect_reason = "heartbeat_timeout";
                    break;
                }
                let heartbeat_message = if supports_readiness {
                    GatewayToAgent::ConnectionProbe(readiness.challenge(generation))
                } else { GatewayToAgent::Ping };
                if out_tx.try_send(heartbeat_message).is_err() {
                    warn!(device_id=%hello.device_id,generation,"agent heartbeat queue unavailable");
                    disconnect_reason = "heartbeat_queue_full";
                    break;
                }
            },
            maybe_retire = retire_rx.recv() => {
                match maybe_retire {
                    Some(()) => {
                        warn!(device_id=%hello.device_id,generation,"agent connection retired after failed control-path health check");
                        disconnect_reason = "control_path_timeout";
                    }
                    // The sender is dropped when a newer connection replaces this
                    // entry in the device map. That is a supersession, not a
                    // failed health check, and the generation guard below keeps
                    // it from touching the replacement.
                    None => info!(device_id=%hello.device_id,generation,"agent connection superseded by a newer connection"),
                }
                break;
            },
            maybe_out=out_rx.recv()=>match maybe_out{Some(message)=>{
                match tokio::time::timeout(Duration::from_secs(5),send_json(&mut socket,&message)).await {
                    Ok(Ok(()))=>{},
                    Ok(Err(e))=>{warn!(device_id=%hello.device_id,error=%e,"failed to send to agent");disconnect_reason="send_error";break},
                    Err(_)=>{warn!(device_id=%hello.device_id,generation,"timed out sending to agent");disconnect_reason="send_timeout";break},
                }
            },None=>break},
            incoming=socket.recv()=>match incoming{
                Some(Ok(Message::Text(text)))=>{
                    last_inbound=Instant::now();
                    match serde_json::from_str::<AgentToGateway>(text.as_str()){
                        Ok(AgentToGateway::Response(response))=>state.pending.complete(&hello.device_id, generation, response),
                        Ok(AgentToGateway::Reconcile(snapshot)) => {
                            let hash = snapshot.content_hash();
                            let changed = {
                                let mut devices = state.devices.write().await;
                                match devices.get_mut(&hello.device_id) {
                                    Some(device) if device.generation == generation => {
                                        let changed = !supports_reconcile_hash
                                            || device.reconcile_hash.as_deref()
                                                != Some(hash.as_str());
                                        device.reconcile = snapshot.clone();
                                        device.reconcile_hash = Some(hash);
                                        changed
                                    }
                                    // A superseded connection must not drive the dashboard.
                                    _ => false,
                                }
                            };
                            if changed {
                                let _ = state.store.record_reconcile(&hello.device_id, &snapshot);
                                state.emit("sessions_changed", Some(hello.device_id.clone()));
                            } else {
                                let _ = state.store.touch_device_seen(&hello.device_id);
                            }
                        }
                        Ok(AgentToGateway::ReconcileUnchanged { hash }) => {
                            let stored = {
                                let devices = state.devices.read().await;
                                match devices.get(&hello.device_id) {
                                    Some(device) if device.generation == generation => {
                                        Some(device.reconcile_hash.as_deref() == Some(hash.as_str()))
                                    }
                                    _ => None,
                                }
                            };
                            match stored {
                                Some(true) => {
                                    let _ = state.store.touch_device_seen(&hello.device_id);
                                }
                                Some(false) => {
                                    // The agent says nothing changed but our view
                                    // disagrees. The stored snapshot is the last
                                    // full one, so refresh the dashboard from it
                                    // and keep the stored hash: the next full
                                    // snapshot repairs the divergence.
                                    warn!(device_id=%hello.device_id,generation,"unchanged reconcile does not match the stored snapshot hash");
                                    state.emit("sessions_changed", Some(hello.device_id.clone()));
                                    let _ = state.store.touch_device_seen(&hello.device_id);
                                }
                                None => {}
                            }
                        },
                        Ok(AgentToGateway::ConnectionProbeReply(reply)) => {
                            if supports_readiness && readiness.accept(&reply) {
                                // Hold the routing read lock until confirmation is queued;
                                // a cached/superseded socket cannot certify itself as active.
                                let devices = state.devices.read().await;
                                if devices.get(&hello.device_id).is_some_and(|device| device.generation == generation) {
                                    if out_tx.try_send(GatewayToAgent::ConnectionReady(reply)).is_err() {
                                        disconnect_reason = "readiness_queue_full";
                                        break;
                                    }
                                } else {
                                    disconnect_reason = "superseded";
                                    break;
                                }
                            }
                        },
                        Ok(AgentToGateway::Pong)=>{}, Ok(_)=>warn!(device_id=%hello.device_id,"unexpected post-auth agent frame"), Err(e)=>warn!(device_id=%hello.device_id,error=%e,"invalid agent frame")
                    }
                },
                Some(Ok(Message::Ping(payload)))=>{last_inbound=Instant::now();if socket.send(Message::Pong(payload)).await.is_err(){break}},
                Some(Ok(Message::Pong(_)))=>{last_inbound=Instant::now();},
                Some(Ok(Message::Close(_)))|None=>{disconnect_reason="socket_closed";break},Some(Ok(_))=>{},Some(Err(e))=>{warn!(device_id=%hello.device_id,error=%e,"agent websocket error");disconnect_reason="socket_error";break}
            }
        }
    }
    state.pending.disconnect(&hello.device_id, generation);
    let mut devices = state.devices.write().await;
    if devices
        .get(&hello.device_id)
        .is_some_and(|d| d.generation == generation)
    {
        devices.remove(&hello.device_id);
        let _ = state
            .store
            .record_disconnect(&hello.device_id, disconnect_reason);
        let _ =
            state
                .store
                .record_activity(Some(&hello.device_id), "device.disconnect", "ok", None);
        state.emit("device_offline", Some(hello.device_id.clone()));
        info!(device_id=%hello.device_id,generation,"agent offline")
    } else {
        info!(device_id=%hello.device_id,generation,"stale agent connection retired")
    }
}

pub(crate) struct AuthenticatedHello {
    hello: DeviceHello,
    public_key: Option<Vec<u8>>,
}

pub(crate) async fn authenticate_agent(
    socket: &mut WebSocket,
    state: &AppState,
) -> Result<AuthenticatedHello, String> {
    let first = recv_agent(socket, Duration::from_secs(5)).await?;
    match first {
        AgentToGateway::AuthStart(start) => authenticate_signed(socket, state, start).await,
        AgentToGateway::LegacyHello(LegacyDeviceHello { hello, token }) => {
            let Some(expected) = &state.legacy_agent_token else {
                return Err("legacy authentication disabled".into());
            };
            if token.as_str() != expected.as_ref() {
                return Err("invalid legacy credential".into());
            }
            if hello.protocol != PROTOCOL_VERSION {
                return Err("protocol mismatch".into());
            }
            Ok(AuthenticatedHello {
                hello,
                public_key: None,
            })
        }
        _ => Err("expected auth_start or legacy_hello".into()),
    }
}

pub(crate) async fn authenticate_signed(
    socket: &mut WebSocket,
    state: &AppState,
    start: DeviceAuthStart,
) -> Result<AuthenticatedHello, String> {
    if start.protocol != PROTOCOL_VERSION {
        return Err("protocol mismatch".into());
    }
    let record = state
        .store
        .get_device(&start.device_id)
        .map_err(|e| e.to_string())?
        .ok_or("unknown device")?;
    if record.revoked {
        return Err("device revoked".into());
    }
    let public_key = record
        .public_key
        .ok_or("device has no enrolled public key")?;
    let mut nonce_bytes = [0_u8; 32];
    getrandom::fill(&mut nonce_bytes).map_err(|_| "random source unavailable")?;
    let nonce = URL_SAFE_NO_PAD.encode(nonce_bytes);
    let expires = now_ms() + 5_000;
    send_json(
        socket,
        &GatewayToAgent::AuthChallenge(AuthChallenge {
            nonce: nonce.clone(),
            gateway_epoch: state.gateway_epoch,
            expires_at_unix_ms: expires,
        }),
    )
    .await
    .map_err(|e| e.to_string())?;
    let AgentToGateway::SignedHello(SignedDeviceHello { hello, signature }) =
        recv_agent(socket, Duration::from_secs(5)).await?
    else {
        return Err("expected signed_hello".into());
    };
    if now_ms() > expires
        || hello.protocol != PROTOCOL_VERSION
        || hello.device_id != start.device_id
        || hello.agent_instance_id != start.agent_instance_id
    {
        return Err("signed hello does not match authentication start".into());
    }
    let key_bytes = <[u8; 32]>::try_from(public_key.as_slice())
        .map_err(|_| "stored public key has invalid length")?;
    let verifying =
        VerifyingKey::from_bytes(&key_bytes).map_err(|_| "stored public key invalid")?;
    let sig_bytes = <[u8; 64]>::try_from(
        URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| "signature encoding invalid")?,
    )
    .map_err(|_| "signature length invalid")?;
    let sig = Signature::from_bytes(&sig_bytes);
    verifying
        .verify_strict(
            &signed_auth_message(&nonce, state.gateway_epoch, &hello),
            &sig,
        )
        .map_err(|_| "signature verification failed")?;
    Ok(AuthenticatedHello {
        hello,
        public_key: Some(public_key),
    })
}

pub(crate) async fn recv_agent(
    socket: &mut WebSocket,
    timeout: Duration,
) -> Result<AgentToGateway, String> {
    match tokio::time::timeout(timeout, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            serde_json::from_str(text.as_str()).map_err(|e| e.to_string())
        }
        Ok(Some(Err(e))) => Err(e.to_string()),
        Ok(Some(Ok(_))) => Err("unexpected websocket frame".into()),
        Ok(None) => Err("connection closed".into()),
        Err(_) => Err("handshake timeout".into()),
    }
}

pub(crate) async fn send_json(
    socket: &mut WebSocket,
    message: &GatewayToAgent,
) -> anyhow::Result<()> {
    socket
        .send(Message::Text(serde_json::to_string(message)?.into()))
        .await?;
    Ok(())
}
