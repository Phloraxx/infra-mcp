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
    let generation = state.next_generation.fetch_add(1, Ordering::Relaxed);
    let (out_tx, mut out_rx) = mpsc::channel::<GatewayToAgent>(64);
    let (retire_tx, mut retire_rx) = mpsc::channel::<()>(1);
    let ack = GatewayToAgent::HelloAck(HelloAck {
        protocol: PROTOCOL_VERSION,
        connection_generation: generation,
        gateway_epoch: state.gateway_epoch,
        control_frame_limit: DEFAULT_CONTROL_FRAME_LIMIT,
        reconcile_requested: true,
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
    {
        let mut devices = state.devices.write().await;
        devices.insert(
            hello.device_id.clone(),
            DeviceConnection {
                hello: hello.clone(),
                generation,
                tx: out_tx.clone(),
                retire: retire_tx,
                reconcile: ReconcileSnapshot::default(),
            },
        );
    }
    let _ = state
        .store
        .record_activity(Some(&hello.device_id), "device.connect", "ok", None);
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
                if last_inbound.elapsed() >= agent_heartbeat_timeout() {
                    warn!(device_id=%hello.device_id,generation,"agent heartbeat timed out; retiring stale connection");
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
            _ = retire_rx.recv() => {
                warn!(device_id=%hello.device_id,generation,"agent connection retired after failed control-path health check");
                disconnect_reason = "control_path_timeout";
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
                            if let Some(device) = state.devices.write().await.get_mut(&hello.device_id)
                                && device.generation == generation
                            {
                                device.reconcile = snapshot.clone();
                            }
                            let _ = state.store.record_reconcile(&hello.device_id, &snapshot);
                            state.emit("sessions_changed", Some(hello.device_id.clone()));
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
