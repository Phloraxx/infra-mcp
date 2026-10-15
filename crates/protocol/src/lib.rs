//! Versioned wire types shared by the Infra gateway and device agent.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 1;
pub const DEFAULT_CONTROL_FRAME_LIMIT: usize = 1024 * 1024;
pub const DEFAULT_READ_LIMIT: usize = 64 * 1024;
pub const MAX_READ_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceHello {
    pub protocol: u16,
    pub device_id: String,
    pub device_name: String,
    pub agent_version: String,
    pub platform: String,
    pub arch: String,
    pub boot_id: String,
    pub agent_instance_id: Uuid,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceAuthStart {
    pub protocol: u16,
    pub device_id: String,
    pub agent_instance_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthChallenge {
    pub nonce: String,
    pub gateway_epoch: Uuid,
    pub expires_at_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedDeviceHello {
    pub hello: DeviceHello,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyDeviceHello {
    pub hello: DeviceHello,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelloAck {
    pub protocol: u16,
    pub connection_generation: u64,
    pub gateway_epoch: Uuid,
    pub control_frame_limit: usize,
    pub reconcile_requested: bool,
}

/// Bytes signed by a device after receiving a fresh gateway challenge.
///
/// The challenge is delivered over the just-opened TLS/WebSocket connection and
/// makes a captured signature unusable on another connection. Length prefixes
/// avoid ambiguous concatenation and keep the format independent of JSON
/// serialization details.
pub fn signed_auth_message(nonce: &str, gateway_epoch: Uuid, hello: &DeviceHello) -> Vec<u8> {
    fn field(out: &mut Vec<u8>, value: &[u8]) {
        out.extend_from_slice(&(value.len() as u32).to_be_bytes());
        out.extend_from_slice(value);
    }

    let mut out = b"infra-v1-device-auth\0".to_vec();
    field(&mut out, nonce.as_bytes());
    field(&mut out, gateway_epoch.as_bytes());
    field(&mut out, &hello.protocol.to_be_bytes());
    field(&mut out, hello.device_id.as_bytes());
    field(&mut out, hello.agent_instance_id.as_bytes());
    field(&mut out, hello.boot_id.as_bytes());
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AgentOperation {
    Status,
    Exec {
        command: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    Files(FileRequest),
    Search(SearchRequest),
    Processes(ProcessRequest),
    Jobs(JobRequest),
    Terminal(TerminalRequest),
    Transfer(TransferRequest),
    System(SystemRequest),
    Network(NetworkRequest),
    Services(ServiceRequest),
    Containers(ContainerRequest),
    Git(GitRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemRequest {
    pub action: SystemAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SystemAction {
    Summary,
    Disks,
    Networks,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkRequest {
    pub action: NetworkAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum NetworkAction {
    Resolve {
        host: String,
    },
    TcpCheck {
        host: String,
        port: u16,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    Listeners,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceRequest {
    pub action: ServiceAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ServiceAction {
    List {
        #[serde(default)]
        query: Option<String>,
    },
    Status {
        name: String,
    },
    Start {
        name: String,
    },
    Stop {
        name: String,
    },
    Restart {
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerRequest {
    pub action: ContainerAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ContainerAction {
    List,
    Inspect {
        container: String,
    },
    Logs {
        container: String,
        #[serde(default)]
        tail: Option<usize>,
    },
    Start {
        container: String,
    },
    Stop {
        container: String,
    },
    Restart {
        container: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitRequest {
    pub action: GitAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GitAction {
    Status {
        repo: String,
    },
    Diff {
        repo: String,
        #[serde(default)]
        staged: bool,
    },
    Log {
        repo: String,
        #[serde(default)]
        limit: Option<usize>,
    },
    Branches {
        repo: String,
    },
    Remotes {
        repo: String,
    },
    Fetch {
        repo: String,
        #[serde(default)]
        remote: Option<String>,
    },
    Pull {
        repo: String,
        #[serde(default)]
        remote: Option<String>,
        #[serde(default)]
        branch: Option<String>,
    },
    Push {
        repo: String,
        #[serde(default)]
        remote: Option<String>,
        #[serde(default)]
        branch: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileRequest {
    pub action: FileAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum FileAction {
    Read {
        path: String,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        length: Option<usize>,
    },
    Write {
        path: String,
        content: String,
        #[serde(default)]
        create_parents: bool,
    },
    Append {
        path: String,
        content: String,
        #[serde(default)]
        create_parents: bool,
    },
    ExactEdit {
        path: String,
        old: String,
        new: String,
        #[serde(default)]
        expected_replacements: Option<usize>,
    },
    Hash {
        path: String,
    },
    List {
        path: String,
    },
    Stat {
        path: String,
    },
    Mkdir {
        path: String,
    },
    Copy {
        source: String,
        destination: String,
        #[serde(default)]
        recursive: bool,
    },
    Move {
        source: String,
        destination: String,
    },
    Trash {
        path: String,
    },
    DeletePermanent {
        path: String,
        #[serde(default)]
        recursive: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchRequest {
    pub root: String,
    pub query: String,
    pub mode: SearchMode,
    #[serde(default)]
    pub literal: bool,
    #[serde(default)]
    pub ignore_case: bool,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub file_pattern: Option<String>,
    #[serde(default)]
    pub max_results: Option<usize>,
    #[serde(default)]
    pub context_lines: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Files,
    Content,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessRequest {
    pub action: ProcessAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ProcessAction {
    List {
        #[serde(default)]
        query: Option<String>,
    },
    Terminate {
        pid: u32,
        #[serde(default)]
        force: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobRequest {
    pub action: JobAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum JobAction {
    Start {
        command: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    List,
    Read {
        job_id: Uuid,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        length: Option<usize>,
    },
    Cancel {
        job_id: Uuid,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransferRequest {
    pub action: TransferAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TransferAction {
    UploadBegin {
        path: String,
        size: u64,
        #[serde(default)]
        sha256: Option<String>,
        #[serde(default)]
        overwrite: bool,
        #[serde(default)]
        create_parents: bool,
    },
    UploadChunk {
        transfer_id: Uuid,
        offset: u64,
        data_base64: String,
    },
    UploadCommit {
        transfer_id: Uuid,
    },
    DownloadBegin {
        path: String,
    },
    DownloadChunk {
        transfer_id: Uuid,
        offset: u64,
        #[serde(default)]
        length: Option<usize>,
    },
    Status {
        transfer_id: Uuid,
    },
    List,
    Cancel {
        transfer_id: Uuid,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerminalRequest {
    pub action: TerminalAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TerminalAction {
    Open {
        command: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        rows: Option<u16>,
        #[serde(default)]
        cols: Option<u16>,
    },
    List,
    Read {
        session_id: Uuid,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        length: Option<usize>,
    },
    Write {
        session_id: Uuid,
        data: String,
    },
    Resize {
        session_id: Uuid,
        rows: u16,
        cols: u16,
    },
    Close {
        session_id: Uuid,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRequest {
    pub id: Uuid,
    pub operation: AgentOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OperationResult {
    Ok { body: serde_json::Value },
    Failed { error: String },
    Canceled { error: Option<String> },
    DeadlineExceeded,
    OutcomeUnknown { error: String },
    Unsupported { operation: String },
    Unauthorized,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentResponse {
    pub id: Uuid,
    pub result: OperationResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileKind {
    Job,
    Terminal,
    Transfer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconcileItem {
    pub id: Uuid,
    pub kind: ReconcileKind,
    pub status: String,
    pub boot_id: String,
    pub started_at_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ReconcileSnapshot {
    pub items: Vec<ReconcileItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum AgentToGateway {
    AuthStart(DeviceAuthStart),
    SignedHello(SignedDeviceHello),
    LegacyHello(LegacyDeviceHello),
    Response(AgentResponse),
    Reconcile(ReconcileSnapshot),
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum GatewayToAgent {
    AuthChallenge(AuthChallenge),
    HelloAck(HelloAck),
    Request(AgentRequest),
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> DeviceHello {
        DeviceHello {
            protocol: PROTOCOL_VERSION,
            device_id: "device-id".into(),
            device_name: "Test".into(),
            agent_version: "0.1.0".into(),
            platform: "linux".into(),
            arch: "aarch64".into(),
            boot_id: "boot".into(),
            agent_instance_id: Uuid::nil(),
            capabilities: vec!["exec".into()],
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn hello_round_trip() {
        let message = AgentToGateway::SignedHello(SignedDeviceHello {
            hello: hello(),
            signature: "signature".into(),
        });
        let json = serde_json::to_string(&message).unwrap();
        let decoded: AgentToGateway = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn auth_message_is_deterministic_and_context_bound() {
        let h = hello();
        let epoch = Uuid::new_v4();
        let first = signed_auth_message("nonce", epoch, &h);
        let second = signed_auth_message("nonce", epoch, &h);
        assert_eq!(first, second);
        assert_ne!(first, signed_auth_message("other", epoch, &h));
    }
}
