//! Process fixtures: a real gateway, a real agent and the proxy between them.

use crate::{EventStream, FaultProxy, McpClient, StoreProbe};
use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use std::{
    fs,
    net::{SocketAddr, TcpListener as StdTcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::OnceLock,
    time::Duration,
};
use tempfile::TempDir;

/// The gateway's heartbeat deadline floor. The harness sets it explicitly so a
/// stalled connection is retired in tens of seconds instead of the 90s default.
pub const HEARTBEAT_TIMEOUT_SECS: u64 = 30;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn target_dir() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            if dir.is_absolute() {
                dir
            } else {
                workspace_root().join(dir)
            }
        }
        None => workspace_root().join("target"),
    }
}

/// Build the two real binaries once per test process. `cargo test --workspace`
/// does not guarantee the plain binaries exist, so the harness asks cargo for
/// them explicitly; a warm target directory makes this a no-op.
fn binary(name: &str) -> PathBuf {
    static BUILD: OnceLock<Result<(), String>> = OnceLock::new();
    let path = target_dir()
        .join("debug")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let result = BUILD.get_or_init(|| {
        let output = Command::new(env!("CARGO"))
            .args(["build", "-p", "infra-gateway", "-p", "infra-agent"])
            .current_dir(workspace_root())
            .output()
            .map_err(|error| format!("spawn cargo build: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "cargo build -p infra-gateway -p infra-agent failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(())
    });
    if let Err(error) = result {
        panic!("{error}");
    }
    assert!(path.exists(), "missing harness binary {}", path.display());
    path
}

fn free_port() -> u16 {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("reserve a loopback port");
    listener.local_addr().expect("loopback address").port()
}

/// Strip inherited `INFRA_*` configuration so a live gateway or agent setup can
/// never leak into a fixture.
fn scrub_environment(command: &mut Command) {
    for (key, _) in std::env::vars() {
        if key.starts_with("INFRA_") {
            command.env_remove(key);
        }
    }
}

pub struct Gateway {
    pub addr: SocketAddr,
    pub base: String,
    pub token: String,
    pub db_path: PathBuf,
    pub log_path: PathBuf,
    child: Option<Child>,
    environment: Vec<(String, String)>,
}

impl Gateway {
    pub fn start(dir: &Path, heartbeat_secs: u64) -> anyhow::Result<Self> {
        let addr = SocketAddr::from(([127, 0, 0, 1], free_port()));
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let db_path = dir.join("gateway.db");
        let log_path = dir.join("gateway.log");
        let environment = vec![
            ("INFRA_GATEWAY_BIND".to_owned(), addr.to_string()),
            ("INFRA_DB_PATH".to_owned(), db_path.display().to_string()),
            ("INFRA_ADMIN_TOKEN".to_owned(), token.clone()),
            (
                "INFRA_HEARTBEAT_TIMEOUT_SECS".to_owned(),
                heartbeat_secs.to_string(),
            ),
            ("RUST_LOG".to_owned(), "info".to_owned()),
        ];
        let mut gateway = Self {
            addr,
            base: format!("http://{addr}"),
            token,
            db_path,
            log_path,
            child: None,
            environment,
        };
        gateway.spawn()?;
        Ok(gateway)
    }

    fn spawn(&mut self) -> anyhow::Result<()> {
        let log = fs::File::create(&self.log_path).context("create gateway log")?;
        let mut command = Command::new(binary("infra-gateway"));
        scrub_environment(&mut command);
        for (key, value) in &self.environment {
            command.env(key, value);
        }
        command
            .current_dir(self.log_path.parent().unwrap_or(Path::new(".")))
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        self.child = Some(command.spawn().context("spawn infra-gateway")?);
        Ok(())
    }

    pub async fn wait_healthy(&self) -> anyhow::Result<()> {
        let client = reqwest::Client::new();
        for _ in 0..400 {
            if let Ok(response) = client.get(format!("{}/healthz", self.base)).send().await
                && response.status().is_success()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err(anyhow!(
            "gateway did not become healthy on {}:\n{}",
            self.addr,
            self.log()
        ))
    }

    /// Restart only this fixture's gateway. The live service is never touched.
    pub async fn restart(&mut self) -> anyhow::Result<()> {
        self.stop();
        self.spawn()?;
        self.wait_healthy().await
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn log(&self) -> String {
        fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    pub async fn wait_for_log(&self, needle: &str, timeout: Duration) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.log().contains(needle) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "gateway log never contained {needle:?}:\n{}",
                    self.log()
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct Agent {
    pub state_dir: PathBuf,
    pub log_path: PathBuf,
    child: Option<Child>,
}

impl Agent {
    pub fn spawn(
        state_dir: &Path,
        log_path: &Path,
        gateway_ws_url: &str,
        enrollment_code: Option<&str>,
        device_name: &str,
    ) -> anyhow::Result<Self> {
        fs::create_dir_all(state_dir).context("create agent state dir")?;
        let log = fs::File::create(log_path).context("create agent log")?;
        let mut command = Command::new(binary("infra-agent"));
        scrub_environment(&mut command);
        command
            .env("INFRA_GATEWAY_URL", gateway_ws_url)
            .env("INFRA_STATE_DIR", state_dir)
            .env("INFRA_DEVICE_NAME", device_name)
            .env("RUST_LOG", "info")
            .current_dir(state_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        if let Some(code) = enrollment_code {
            command.env("INFRA_ENROLLMENT_CODE", code);
        }
        Ok(Self {
            state_dir: state_dir.to_path_buf(),
            log_path: log_path.to_path_buf(),
            child: Some(command.spawn().context("spawn infra-agent")?),
        })
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn log(&self) -> String {
        fs::read_to_string(&self.log_path).unwrap_or_default()
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One hermetic gateway + proxy + agent fixture.
pub struct Fixture {
    pub dir: TempDir,
    pub gateway: Gateway,
    pub proxy: FaultProxy,
    pub agent: Agent,
    pub device_id: String,
    pub mcp: McpClient,
    pub store: StoreProbe,
    pub events: EventStream,
}

impl Fixture {
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().expect("create fixture directory");
        let gateway = Gateway::start(dir.path(), HEARTBEAT_TIMEOUT_SECS).expect("start gateway");
        gateway.wait_healthy().await.expect("gateway health");
        let events = EventStream::start(&gateway.base, &gateway.token)
            .await
            .expect("subscribe to dashboard events");
        let proxy = FaultProxy::start(gateway.addr)
            .await
            .expect("start fault proxy");
        let mcp = McpClient::new(&gateway.base, &gateway.token);
        let code = create_enrollment(&gateway)
            .await
            .expect("create enrollment");
        let agent = Agent::spawn(
            &dir.path().join("agent"),
            &dir.path().join("agent.log"),
            &proxy.agent_url(),
            Some(&code),
            "e2e-fixture",
        )
        .expect("start agent");
        let store = StoreProbe::new(gateway.db_path.clone());
        let device_id = wait_for_online(&mcp).await;
        Self {
            dir,
            gateway,
            proxy,
            agent,
            device_id,
            mcp,
            store,
            events,
        }
    }

    /// Spawn a second agent process with the same identity, which produces a
    /// second live connection for one device.
    pub fn spawn_second_agent(&self) -> anyhow::Result<Agent> {
        Agent::spawn(
            &self.agent.state_dir,
            &self.dir.path().join("agent-second.log"),
            &self.proxy.agent_url(),
            None,
            "e2e-fixture",
        )
    }

    pub async fn device(&self) -> Value {
        let devices = self.devices().await;
        devices
            .into_iter()
            .find(|device| device["device_id"] == json!(self.device_id))
            .unwrap_or_else(|| panic!("device {} disappeared", self.device_id))
    }

    pub async fn devices(&self) -> Vec<Value> {
        let response = reqwest::Client::new()
            .get(format!("{}/api/devices", self.gateway.base))
            .bearer_auth(&self.gateway.token)
            .send()
            .await
            .expect("list devices");
        assert!(
            response.status().is_success(),
            "device listing failed: {}",
            response.status()
        );
        response.json().await.expect("decode device listing")
    }

    pub async fn generation(&self) -> u64 {
        self.device().await["connection_generation"]
            .as_u64()
            .unwrap_or(0)
    }

    pub async fn wait_online(&self) -> u64 {
        self.wait_for_generation_above(0).await
    }

    /// Wait until the device is online on a generation newer than `after`.
    pub async fn wait_for_generation_above(&self, after: u64) -> u64 {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let device = self.device().await;
            let generation = device["connection_generation"].as_u64().unwrap_or(0);
            if device["online"].as_bool().unwrap_or(false) && generation > after {
                return generation;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "device {} never came online above generation {after}: {device}",
                    self.device_id
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn wait_active_jobs(&self, expected: u64) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let device = self.device().await;
            if device["active_jobs"].as_u64() == Some(expected) {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "device {} never reported {expected} active jobs: {device}",
                    self.device_id
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

async fn create_enrollment(gateway: &Gateway) -> anyhow::Result<String> {
    let response = reqwest::Client::new()
        .post(format!("{}/api/enrollments", gateway.base))
        .bearer_auth(&gateway.token)
        .json(&json!({"ttl_seconds": 300}))
        .send()
        .await
        .context("create enrollment request")?;
    let status = response.status();
    let body: Value = response.json().await.context("decode enrollment")?;
    if !status.is_success() {
        return Err(anyhow!("enrollment creation failed ({status}): {body}"));
    }
    body["code"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("enrollment response has no code: {body}"))
}

async fn wait_for_online(mcp: &McpClient) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(result) = mcp.call("devices", json!({})).await
            && let Some(devices) = result["devices"].as_array()
            && let Some(device) = devices
                .iter()
                .find(|device| device["online"].as_bool() == Some(true))
            && let Some(device_id) = device["device_id"].as_str()
        {
            return device_id.to_owned();
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("agent never became online");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
