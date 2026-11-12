//! Versioned wire types shared by the Infra gateway and device agent.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 1;
pub const DEFAULT_CONTROL_FRAME_LIMIT: usize = 8 * 1024 * 1024;
pub const DEFAULT_READ_LIMIT: usize = 64 * 1024;
pub const MAX_READ_LIMIT: usize = 1024 * 1024;
pub const FILE_LIST_PAGINATION_CAPABILITY: &str = "files.list_page";
pub const MAX_DIRECTORY_PAGE_SIZE: usize = 1000;
pub const MAX_DIRECTORY_ENTRIES: usize = 100_000;
pub const MAX_DIRECTORY_PAGE_BYTES: usize = 512 * 1024;
pub const CONNECTION_READINESS_CAPABILITY: &str = "connection.readiness_v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionProbe {
    pub nonce: Uuid,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionProbeReply {
    pub probe: ConnectionProbe,
    pub pid: u32,
    pub protocol: u16,
}

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
    Desktop(DesktopRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopRequest {
    pub action: DesktopAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DesktopAction {
    Permissions,
    Displays,
    Windows,
    Screenshot {
        #[serde(default)]
        display: Option<u32>,
        #[serde(default)]
        x: Option<i32>,
        #[serde(default)]
        y: Option<i32>,
        #[serde(default)]
        width: Option<u32>,
        #[serde(default)]
        height: Option<u32>,
    },
    Focus {
        app: String,
    },
    Click {
        x: i32,
        y: i32,
        #[serde(default)]
        button: Option<String>,
    },
    TypeText {
        text: String,
    },
    Key {
        key: String,
        #[serde(default)]
        modifiers: Vec<String>,
    },
    ClipboardGet,
    ClipboardSet {
        text: String,
    },
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
        /// Entry offset in native filename order, not a byte offset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<u64>,
        /// Maximum entries. Requires files.list_page when explicitly supplied.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        length: Option<usize>,
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
    Wait {
        job_id: Uuid,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        length: Option<usize>,
        #[serde(default)]
        wait_timeout_ms: Option<u64>,
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
    ConnectionProbeReply(ConnectionProbeReply),
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum GatewayToAgent {
    AuthChallenge(AuthChallenge),
    HelloAck(HelloAck),
    Request(AgentRequest),
    ConnectionProbe(ConnectionProbe),
    ConnectionReady(ConnectionProbeReply),
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

    #[test]
    fn readiness_frames_roundtrip_without_changing_legacy_heartbeats() {
        assert_eq!(
            serde_json::to_value(GatewayToAgent::Ping).unwrap(),
            serde_json::json!({"type":"ping"})
        );
        assert_eq!(
            serde_json::to_value(AgentToGateway::Pong).unwrap(),
            serde_json::json!({"type":"pong"})
        );
        let probe = ConnectionProbe {
            nonce: Uuid::new_v4(),
            generation: 7,
        };
        let reply = ConnectionProbeReply {
            probe: probe.clone(),
            pid: 42,
            protocol: PROTOCOL_VERSION,
        };
        for message in [
            GatewayToAgent::ConnectionProbe(probe),
            GatewayToAgent::ConnectionReady(reply.clone()),
        ] {
            let value = serde_json::to_value(&message).unwrap();
            assert_eq!(
                serde_json::from_value::<GatewayToAgent>(value).unwrap(),
                message
            );
        }
        let response = AgentToGateway::ConnectionProbeReply(reply);
        assert_eq!(
            serde_json::from_value::<AgentToGateway>(serde_json::to_value(&response).unwrap())
                .unwrap(),
            response
        );
        assert!(
            serde_json::from_value::<GatewayToAgent>(
                serde_json::json!({"type":"connection_probe","body":{"generation":7}})
            )
            .is_err()
        );
    }

    #[test]
    fn android_files_operation_uses_nested_action_json() {
        let request = AgentRequest {
            id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
            operation: AgentOperation::Files(FileRequest {
                action: FileAction::List {
                    path: String::from("site"),
                    offset: None,
                    length: None,
                },
            }),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["operation"]["op"], "files");
        assert_eq!(json["operation"]["action"]["action"], "list");
        assert_eq!(json["operation"]["action"]["path"], "site");
        assert!(json["operation"]["action"].get("offset").is_none());
        assert!(json["operation"]["action"].get("length").is_none());
        let decoded: AgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn directory_list_optional_fields_preserve_legacy_wire_shape() {
        let legacy: FileAction =
            serde_json::from_value(serde_json::json!({"action":"list","path":"fixture"})).unwrap();
        assert_eq!(
            legacy,
            FileAction::List {
                path: "fixture".into(),
                offset: None,
                length: None
            }
        );
        let paged = FileAction::List {
            path: "fixture".into(),
            offset: Some(42),
            length: Some(10),
        };
        let wire = serde_json::to_value(&paged).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({"action":"list","path":"fixture","offset":42,"length":10})
        );
        assert_eq!(serde_json::from_value::<FileAction>(wire).unwrap(), paged);
    }

    #[test]
    fn android_system_operation_uses_nested_action_json() {
        let request = AgentRequest {
            id: Uuid::new_v4(),
            operation: AgentOperation::System(SystemRequest {
                action: SystemAction::Summary,
            }),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["operation"]["op"], "system");
        assert_eq!(json["operation"]["action"]["action"], "summary");
        let decoded: AgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn android_java_signed_payload_golden() {
        use sha2::{Digest, Sha256};
        let mut h = hello();
        h.device_id = "android-fixture".into();
        h.agent_instance_id = Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap();
        h.boot_id = "boot-fixture".into();
        let epoch = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        let payload = signed_auth_message("signed-nonce", epoch, &h);
        assert_eq!(payload.len(), 118);
        assert_eq!(
            format!("{:x}", Sha256::digest(&payload)),
            "ee24049221d4c59b6d8da0914704309e01b9d205bc5f18599edccbd34a02783a"
        );
    }

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
