//! Loopback TCP proxy with injectable faults.
//!
//! The proxy sits between the agent and the gateway. Faults are per connection
//! so a scenario can break the current link while the replacement link stays
//! healthy, which is what the supersession and heartbeat scenarios need.

use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        TcpListener, TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::watch,
};

/// Faults applied to one proxied connection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Faults {
    /// Drop everything the agent sends. The gateway treats any inbound frame as
    /// liveness, so a stalled agent is modelled by dropping the whole direction:
    /// dropping only pongs would leave the opportunistic reconcile keeping the
    /// connection alive and the heartbeat deadline would never fire.
    pub blackhole_upstream: bool,
    /// Delay each forwarded gateway-to-agent chunk.
    pub delay_downstream_ms: u64,
    /// Write forwarded bytes in chunks of this size; zero forwards whole reads.
    pub split_bytes: usize,
}

struct ConnState {
    faults: Mutex<Faults>,
    cut: watch::Sender<bool>,
    agent_close: watch::Sender<bool>,
}

impl ConnState {
    fn new() -> Self {
        Self {
            faults: Mutex::new(Faults::default()),
            cut: watch::channel(false).0,
            agent_close: watch::channel(false).0,
        }
    }

    fn faults(&self) -> Faults {
        *self.faults.lock().expect("fault state poisoned")
    }
}

struct Inner {
    upstream: SocketAddr,
    next_id: AtomicU64,
    conns: Mutex<HashMap<u64, Arc<ConnState>>>,
    newest: watch::Sender<u64>,
}

/// Handle to the running proxy. Dropping it leaves the accept loop running for
/// the lifetime of the test process; every fixture owns its own listener.
pub struct FaultProxy {
    addr: SocketAddr,
    inner: Arc<Inner>,
}

impl FaultProxy {
    pub async fn start(upstream: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let (newest, _) = watch::channel(0);
        let inner = Arc::new(Inner {
            upstream,
            next_id: AtomicU64::new(1),
            conns: Mutex::new(HashMap::new()),
            newest,
        });
        let accept_inner = inner.clone();
        tokio::spawn(async move {
            loop {
                let Ok((agent, _)) = listener.accept().await else {
                    break;
                };
                let id = accept_inner.next_id.fetch_add(1, Ordering::Relaxed);
                let state = Arc::new(ConnState::new());
                accept_inner
                    .conns
                    .lock()
                    .expect("connection map poisoned")
                    .insert(id, state.clone());
                accept_inner.newest.send_replace(id);
                let inner = accept_inner.clone();
                tokio::spawn(async move {
                    run_connection(agent, inner.upstream, state).await;
                    inner
                        .conns
                        .lock()
                        .expect("connection map poisoned")
                        .remove(&id);
                });
            }
        });
        Ok(Self { addr, inner })
    }

    /// Address the agent must dial instead of the gateway.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// WebSocket URL the agent must dial.
    pub fn agent_url(&self) -> String {
        format!("ws://{}/agent", self.addr)
    }

    /// Identifier of the most recently accepted connection.
    pub fn current_connection(&self) -> u64 {
        *self.inner.newest.borrow()
    }

    /// Wait for a connection newer than `after`.
    pub async fn wait_for_connection(&self, after: u64) -> u64 {
        let mut newest = self.inner.newest.subscribe();
        loop {
            let current = *newest.borrow_and_update();
            if current > after {
                return current;
            }
            if newest.changed().await.is_err() {
                return current;
            }
        }
    }

    pub fn set_faults(&self, connection: u64, faults: Faults) {
        let state = self.state(connection);
        *state.faults.lock().expect("fault state poisoned") = faults;
    }

    /// Close both directions immediately.
    pub fn cut(&self, connection: u64) {
        self.state(connection).cut.send_replace(true);
    }

    /// Close only the agent-facing socket, leaving the upstream socket open.
    pub fn cut_agent_side(&self, connection: u64) {
        self.state(connection).agent_close.send_replace(true);
    }

    fn state(&self, connection: u64) -> Arc<ConnState> {
        self.inner
            .conns
            .lock()
            .expect("connection map poisoned")
            .get(&connection)
            .unwrap_or_else(|| panic!("no proxied connection {connection}"))
            .clone()
    }
}

async fn run_connection(agent: TcpStream, upstream: SocketAddr, state: Arc<ConnState>) {
    let Ok(upstream_stream) = TcpStream::connect(upstream).await else {
        return;
    };
    let _ = agent.set_nodelay(true);
    let _ = upstream_stream.set_nodelay(true);
    let (mut agent_rx, agent_tx) = agent.into_split();
    let (upstream_rx, mut upstream_tx) = upstream_stream.into_split();
    // Forward the HTTP request head one byte at a time so no WebSocket frame is
    // consumed before the raw forwarding loops start.
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") && head.len() < 16 * 1024 {
        let mut byte = [0_u8; 1];
        match agent_rx.read(&mut byte).await {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    if upstream_tx.write_all(&head).await.is_err() {
        return;
    }
    let up = tokio::spawn(upstream_loop(agent_rx, upstream_tx, state.clone()));
    let down = tokio::spawn(downstream_loop(upstream_rx, agent_tx, state.clone()));
    let _ = tokio::join!(up, down);
}

async fn upstream_loop(mut rx: OwnedReadHalf, mut tx: OwnedWriteHalf, state: Arc<ConnState>) {
    let mut cut = state.cut.subscribe();
    let mut buf = vec![0_u8; 32 * 1024];
    loop {
        if *cut.borrow() {
            break;
        }
        tokio::select! {
            _ = cut.changed() => break,
            read = rx.read(&mut buf) => match read {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    let faults = state.faults();
                    if faults.blackhole_upstream {
                        continue;
                    }
                    if write_split(&mut tx, &buf[..count], faults.split_bytes)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    }
    let _ = tx.shutdown().await;
}

async fn downstream_loop(mut rx: OwnedReadHalf, mut tx: OwnedWriteHalf, state: Arc<ConnState>) {
    let mut cut = state.cut.subscribe();
    let mut close = state.agent_close.subscribe();
    let mut buf = vec![0_u8; 32 * 1024];
    loop {
        if *cut.borrow() {
            break;
        }
        tokio::select! {
            _ = cut.changed() => break,
            _ = close.changed() => {
                let _ = tx.shutdown().await;
                break;
            }
            read = rx.read(&mut buf) => match read {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    let faults = state.faults();
                    if faults.delay_downstream_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(faults.delay_downstream_ms)).await;
                    }
                    if write_split(&mut tx, &buf[..count], faults.split_bytes)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    }
}

async fn write_split(tx: &mut OwnedWriteHalf, mut data: &[u8], split: usize) -> io::Result<()> {
    if split == 0 {
        return tx.write_all(data).await;
    }
    while !data.is_empty() {
        let count = split.min(data.len());
        tx.write_all(&data[..count]).await?;
        data = &data[count..];
    }
    Ok(())
}
