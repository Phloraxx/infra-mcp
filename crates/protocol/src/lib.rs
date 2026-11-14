//! Versioned wire types shared by the Infra gateway and device agent.

mod bulk;

pub use bulk::{
    BULK_FRAME_HEADER_LEN, BULK_FRAME_MAGIC, BULK_FRAME_VERSION, BulkFrame, BulkFrameError,
    BulkKind, MAX_BULK_PAYLOAD, OwnedBulkFrame, TERMINAL_BINARY_CAPABILITY,
    TRANSFER_BINARY_CAPABILITY,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
/// Advertised by an agent that can send a compact unchanged reconcile instead of
/// a full snapshot. Additive: the gateway keeps the old behaviour for agents that
/// do not advertise it.
pub const RECONCILE_HASH_CAPABILITY: &str = "reconcile.hash_v1";

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
    /// True when the gateway understands [`AgentToGateway::ReconcileUnchanged`].
    /// Old gateways omit the field, so a new agent keeps sending full snapshots
    /// to them instead of a frame they would reject.
    #[serde(default)]
    pub reconcile_hash: bool,
    /// True when the gateway will carry payload-bearing operations on this
    /// connection in [`BulkFrame`]s. The agent only accepts binary frames after
    /// this acknowledgement; an old gateway omits the field and the agent keeps
    /// the JSON path, which is also what an old agent gets from a new gateway.
    #[serde(default)]
    pub binary_frames: bool,
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
    /// Announcement for a chunk whose bytes follow in a
    /// [`BulkKind::UploadChunk`] frame on the same socket. Only sent to an agent
    /// that advertised [`TRANSFER_BINARY_CAPABILITY`].
    UploadChunkBinary {
        transfer_id: Uuid,
        offset: u64,
        length: usize,
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
    /// Announcement for terminal input whose bytes follow in a
    /// [`BulkKind::TerminalData`] frame on the same socket. Only sent to an agent
    /// that advertised [`TERMINAL_BINARY_CAPABILITY`].
    WriteBinary {
        session_id: Uuid,
        length: usize,
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

impl ReconcileSnapshot {
    /// Stable content hash over the fields the dashboard renders: id, kind,
    /// status, boot_id and started_at. Item order is not content, so the items
    /// are sorted by (kind, id) before hashing; two snapshots that describe the
    /// same sessions always produce the same hash.
    pub fn content_hash(&self) -> String {
        fn field(hasher: &mut Sha256, value: &[u8]) {
            hasher.update((value.len() as u32).to_be_bytes());
            hasher.update(value);
        }
        let mut items: Vec<&ReconcileItem> = self.items.iter().collect();
        items.sort_by(|left, right| {
            (kind_label(&left.kind), left.id).cmp(&(kind_label(&right.kind), right.id))
        });
        let mut hasher = Sha256::new();
        hasher.update(b"infra-v1-reconcile\0");
        hasher.update((items.len() as u32).to_be_bytes());
        for item in items {
            field(&mut hasher, item.id.as_bytes());
            field(&mut hasher, kind_label(&item.kind).as_bytes());
            field(&mut hasher, item.status.as_bytes());
            field(&mut hasher, item.boot_id.as_bytes());
            hasher.update(item.started_at_unix_ms.to_be_bytes());
        }
        hex::encode(hasher.finalize())
    }
}

fn kind_label(kind: &ReconcileKind) -> &'static str {
    match kind {
        ReconcileKind::Job => "job",
        ReconcileKind::Terminal => "terminal",
        ReconcileKind::Transfer => "transfer",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum AgentToGateway {
    AuthStart(DeviceAuthStart),
    SignedHello(SignedDeviceHello),
    LegacyHello(LegacyDeviceHello),
    Response(AgentResponse),
    Reconcile(ReconcileSnapshot),
    /// Compact "nothing changed" reconcile. Only sent after the gateway
    /// acknowledged [`RECONCILE_HASH_CAPABILITY`] in its hello ack.
    ReconcileUnchanged {
        hash: String,
    },
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
            hex::encode(Sha256::digest(&payload)),
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
    #[test]
    fn operation_result_wire_shapes_match_contract() {
        let cases: Vec<(OperationResult, serde_json::Value)> = vec![
            (
                OperationResult::Ok {
                    body: serde_json::json!({"answer": 42}),
                },
                serde_json::json!({"status": "ok", "body": {"answer": 42}}),
            ),
            (
                OperationResult::Failed {
                    error: "boom".into(),
                },
                serde_json::json!({"status": "failed", "error": "boom"}),
            ),
            (
                OperationResult::Canceled { error: None },
                serde_json::json!({"status": "canceled", "error": null}),
            ),
            (
                OperationResult::Canceled {
                    error: Some("stop".into()),
                },
                serde_json::json!({"status": "canceled", "error": "stop"}),
            ),
            (
                OperationResult::DeadlineExceeded,
                serde_json::json!({"status": "deadline_exceeded"}),
            ),
            (
                OperationResult::OutcomeUnknown {
                    error: "lost".into(),
                },
                serde_json::json!({"status": "outcome_unknown", "error": "lost"}),
            ),
            (
                OperationResult::Unsupported {
                    operation: "desktop.screenshot".into(),
                },
                serde_json::json!({"status": "unsupported", "operation": "desktop.screenshot"}),
            ),
            (
                OperationResult::Unauthorized,
                serde_json::json!({"status": "unauthorized"}),
            ),
        ];
        for (result, expected) in cases {
            let wire = serde_json::to_value(&result).unwrap();
            assert_eq!(wire, expected);
            assert_eq!(
                serde_json::from_value::<OperationResult>(wire).unwrap(),
                result
            );
        }
    }

    #[test]
    fn agent_response_envelope_roundtrips_result_bodies() {
        let response = AgentResponse {
            id: Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
            result: OperationResult::Ok {
                body: serde_json::json!({"exit_code": 0, "stdout": "ok"}),
            },
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["result"]["status"], "ok");
        assert_eq!(json["result"]["body"]["stdout"], "ok");
        assert_eq!(
            serde_json::from_value::<AgentResponse>(json).unwrap(),
            response
        );
    }

    #[test]
    fn agent_operation_envelope_tags_match_wire_contract() {
        let cases: Vec<(&str, AgentOperation)> = vec![
            ("status", AgentOperation::Status),
            (
                "exec",
                AgentOperation::Exec {
                    command: "true".into(),
                    cwd: None,
                    timeout_ms: None,
                },
            ),
            (
                "search",
                AgentOperation::Search(SearchRequest {
                    root: "/".into(),
                    query: "q".into(),
                    mode: SearchMode::Files,
                    literal: false,
                    ignore_case: false,
                    include_hidden: false,
                    file_pattern: None,
                    max_results: None,
                    context_lines: None,
                }),
            ),
            (
                "processes",
                AgentOperation::Processes(ProcessRequest {
                    action: ProcessAction::List { query: None },
                }),
            ),
            (
                "jobs",
                AgentOperation::Jobs(JobRequest {
                    action: JobAction::List,
                }),
            ),
            (
                "terminal",
                AgentOperation::Terminal(TerminalRequest {
                    action: TerminalAction::List,
                }),
            ),
            (
                "transfer",
                AgentOperation::Transfer(TransferRequest {
                    action: TransferAction::List,
                }),
            ),
            (
                "system",
                AgentOperation::System(SystemRequest {
                    action: SystemAction::Summary,
                }),
            ),
            (
                "network",
                AgentOperation::Network(NetworkRequest {
                    action: NetworkAction::Listeners,
                }),
            ),
            (
                "services",
                AgentOperation::Services(ServiceRequest {
                    action: ServiceAction::List { query: None },
                }),
            ),
            (
                "containers",
                AgentOperation::Containers(ContainerRequest {
                    action: ContainerAction::List,
                }),
            ),
            (
                "git",
                AgentOperation::Git(GitRequest {
                    action: GitAction::Branches { repo: ".".into() },
                }),
            ),
            (
                "desktop",
                AgentOperation::Desktop(DesktopRequest {
                    action: DesktopAction::Permissions,
                }),
            ),
        ];
        for (tag, operation) in cases {
            let request = AgentRequest {
                id: Uuid::new_v4(),
                operation,
            };
            let json = serde_json::to_value(&request).unwrap();
            assert_eq!(json["operation"]["op"], *tag);
            assert_eq!(
                serde_json::from_value::<AgentRequest>(json).unwrap(),
                request
            );
        }
    }

    #[test]
    fn search_request_wire_shape_and_mode_tags() {
        let request = AgentRequest {
            id: Uuid::new_v4(),
            operation: AgentOperation::Search(SearchRequest {
                root: "/srv".into(),
                query: "needle".into(),
                mode: SearchMode::Content,
                literal: true,
                ignore_case: true,
                include_hidden: false,
                file_pattern: Some("*.rs".into()),
                max_results: Some(10),
                context_lines: Some(2),
            }),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["operation"]["op"], "search");
        assert_eq!(json["operation"]["mode"], "content");
        assert_eq!(
            serde_json::from_value::<AgentRequest>(json).unwrap(),
            request
        );
        let legacy: SearchRequest = serde_json::from_value(serde_json::json!({
            "root": "/srv",
            "query": "q",
            "mode": "files"
        }))
        .unwrap();
        assert!(!legacy.literal && !legacy.ignore_case && !legacy.include_hidden);
        assert_eq!(legacy.file_pattern, None);
        assert_eq!(legacy.max_results, None);
    }

    #[test]
    fn auth_and_hello_ack_frames_roundtrip() {
        let epoch = Uuid::parse_str("00000000-0000-4000-8000-000000000004").unwrap();
        let challenge = GatewayToAgent::AuthChallenge(AuthChallenge {
            nonce: "nonce".into(),
            gateway_epoch: epoch,
            expires_at_unix_ms: 123,
        });
        let ack = GatewayToAgent::HelloAck(HelloAck {
            protocol: PROTOCOL_VERSION,
            connection_generation: 9,
            gateway_epoch: epoch,
            control_frame_limit: DEFAULT_CONTROL_FRAME_LIMIT,
            reconcile_requested: false,
            reconcile_hash: true,
            binary_frames: true,
        });
        for message in [challenge.clone(), ack.clone()] {
            let value = serde_json::to_value(&message).unwrap();
            assert_eq!(
                serde_json::from_value::<GatewayToAgent>(value).unwrap(),
                message
            );
        }
        assert_eq!(
            serde_json::to_value(&challenge).unwrap()["type"],
            "auth_challenge"
        );
        assert_eq!(serde_json::to_value(&ack).unwrap()["type"], "hello_ack");
        assert_eq!(
            serde_json::to_value(&ack).unwrap()["body"]["control_frame_limit"],
            DEFAULT_CONTROL_FRAME_LIMIT
        );
    }

    #[test]
    fn reconcile_snapshot_roundtrips_with_snake_case_kinds() {
        let item = ReconcileItem {
            id: Uuid::new_v4(),
            kind: ReconcileKind::Job,
            status: "running".into(),
            boot_id: "boot".into(),
            started_at_unix_ms: 5,
        };
        let snapshot = ReconcileSnapshot {
            items: vec![item.clone()],
        };
        let wire = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(wire["items"][0]["kind"], "job");
        assert_eq!(
            serde_json::from_value::<ReconcileSnapshot>(wire).unwrap(),
            snapshot
        );
    }

    fn reconcile_item(id: Uuid, kind: ReconcileKind, status: &str) -> ReconcileItem {
        ReconcileItem {
            id,
            kind,
            status: status.into(),
            boot_id: "boot".into(),
            started_at_unix_ms: 5,
        }
    }

    #[test]
    fn reconcile_content_hash_is_order_independent_and_content_sensitive() {
        let job = reconcile_item(Uuid::from_u128(1), ReconcileKind::Job, "running");
        let terminal = reconcile_item(Uuid::from_u128(2), ReconcileKind::Terminal, "running");
        let forward = ReconcileSnapshot {
            items: vec![job.clone(), terminal.clone()],
        };
        let reversed = ReconcileSnapshot {
            items: vec![terminal.clone(), job.clone()],
        };
        assert_eq!(
            forward.content_hash(),
            reversed.content_hash(),
            "item order is not content"
        );
        assert_eq!(forward.content_hash().len(), 64);

        let mut changed_status = forward.clone();
        changed_status.items[0].status = "completed".into();
        assert_ne!(forward.content_hash(), changed_status.content_hash());

        let mut changed_started = forward.clone();
        changed_started.items[0].started_at_unix_ms = 6;
        assert_ne!(forward.content_hash(), changed_started.content_hash());

        let mut changed_boot = forward.clone();
        changed_boot.items[0].boot_id = "other-boot".into();
        assert_ne!(forward.content_hash(), changed_boot.content_hash());

        let mut changed_kind = forward.clone();
        changed_kind.items[0].kind = ReconcileKind::Transfer;
        assert_ne!(forward.content_hash(), changed_kind.content_hash());

        let mut changed_id = forward.clone();
        changed_id.items[0].id = Uuid::from_u128(3);
        assert_ne!(forward.content_hash(), changed_id.content_hash());

        assert_eq!(
            ReconcileSnapshot::default().content_hash(),
            ReconcileSnapshot::default().content_hash()
        );
        assert_ne!(
            ReconcileSnapshot::default().content_hash(),
            forward.content_hash()
        );
    }

    #[test]
    fn reconcile_unchanged_frame_roundtrips_as_a_compact_message() {
        let message = AgentToGateway::ReconcileUnchanged {
            hash: "abc123".into(),
        };
        let wire = serde_json::to_value(&message).unwrap();
        assert_eq!(wire["type"], "reconcile_unchanged");
        assert_eq!(wire["body"]["hash"], "abc123");
        assert!(
            wire["body"].get("items").is_none(),
            "the unchanged frame must not carry a snapshot: {wire}"
        );
        assert_eq!(
            serde_json::from_value::<AgentToGateway>(wire).unwrap(),
            message
        );
    }

    #[test]
    fn hello_ack_reconcile_hash_defaults_to_false_for_old_gateways() {
        let legacy = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "connection_generation": 3,
            "gateway_epoch": Uuid::nil().to_string(),
            "control_frame_limit": DEFAULT_CONTROL_FRAME_LIMIT,
            "reconcile_requested": true
        });
        let ack: HelloAck = serde_json::from_value(legacy).unwrap();
        assert!(
            !ack.reconcile_hash,
            "an old gateway must not be assumed to understand the compact frame"
        );
        assert!(
            !ack.binary_frames,
            "an old gateway must not be assumed to understand bulk frames"
        );
        let current = serde_json::to_value(HelloAck {
            protocol: PROTOCOL_VERSION,
            connection_generation: 3,
            gateway_epoch: Uuid::nil(),
            control_frame_limit: DEFAULT_CONTROL_FRAME_LIMIT,
            reconcile_requested: true,
            reconcile_hash: true,
            binary_frames: true,
        })
        .unwrap();
        assert_eq!(current["reconcile_hash"], true);
        assert_eq!(current["binary_frames"], true);
    }

    #[test]
    fn binary_actions_roundtrip_without_touching_the_json_path() {
        let transfer_id = Uuid::from_u128(7);
        let upload = TransferAction::UploadChunkBinary {
            transfer_id,
            offset: 262_144,
            length: 262_144,
        };
        let wire = serde_json::to_value(&upload).unwrap();
        assert_eq!(wire["action"], "upload_chunk_binary");
        assert_eq!(wire["offset"], 262_144);
        assert_eq!(wire["length"], 262_144);
        assert_eq!(
            serde_json::from_value::<TransferAction>(wire).unwrap(),
            upload
        );

        let legacy = serde_json::json!({
            "action": "upload_chunk",
            "transfer_id": transfer_id.to_string(),
            "offset": 0,
            "data_base64": "AAEC"
        });
        assert_eq!(
            serde_json::from_value::<TransferAction>(legacy).unwrap(),
            TransferAction::UploadChunk {
                transfer_id,
                offset: 0,
                data_base64: "AAEC".into()
            },
            "the base64 action keeps its exact wire shape"
        );

        let write = TerminalAction::WriteBinary {
            session_id: transfer_id,
            length: 12,
        };
        let wire = serde_json::to_value(&write).unwrap();
        assert_eq!(wire["action"], "write_binary");
        assert_eq!(wire["length"], 12);
        assert_eq!(
            serde_json::from_value::<TerminalAction>(wire).unwrap(),
            write
        );
    }

    #[test]
    fn exec_output_roundtrips() {
        let output = ExecOutput {
            exit_code: Some(0),
            stdout: "out".into(),
            stderr: String::new(),
            truncated: false,
        };
        let wire = serde_json::to_value(&output).unwrap();
        assert_eq!(wire["exit_code"], 0);
        assert_eq!(wire["truncated"], false);
        assert_eq!(serde_json::from_value::<ExecOutput>(wire).unwrap(), output);
    }

    #[test]
    fn protocol_constants_pin_the_wire_contract() {
        assert_eq!(PROTOCOL_VERSION, 1);
        assert_eq!(DEFAULT_CONTROL_FRAME_LIMIT, 8 * 1024 * 1024);
        assert_eq!(DEFAULT_READ_LIMIT, 64 * 1024);
        assert_eq!(MAX_READ_LIMIT, 1024 * 1024);
        const _: () = assert!(DEFAULT_READ_LIMIT <= MAX_READ_LIMIT);
        assert_eq!(FILE_LIST_PAGINATION_CAPABILITY, "files.list_page");
        assert_eq!(CONNECTION_READINESS_CAPABILITY, "connection.readiness_v1");
        assert_eq!(RECONCILE_HASH_CAPABILITY, "reconcile.hash_v1");
        assert_eq!(TRANSFER_BINARY_CAPABILITY, "transfer.binary_v1");
        assert_eq!(TERMINAL_BINARY_CAPABILITY, "terminal.binary_v1");
        assert_eq!(BULK_FRAME_MAGIC, b'I');
        assert_eq!(BULK_FRAME_VERSION, 1);
        assert_eq!(BULK_FRAME_HEADER_LEN, 32);
        assert_eq!(MAX_BULK_PAYLOAD, 256 * 1024);
        assert_eq!(MAX_DIRECTORY_PAGE_SIZE, 1000);
        assert_eq!(MAX_DIRECTORY_ENTRIES, 100_000);
        assert_eq!(MAX_DIRECTORY_PAGE_BYTES, 512 * 1024);
        const _: () = assert!(MAX_DIRECTORY_PAGE_SIZE <= MAX_DIRECTORY_ENTRIES);
    }

    #[test]
    fn device_hello_metadata_defaults_preserve_legacy_frames() {
        let wire = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "device_id": "device-id",
            "device_name": "Test",
            "agent_version": "0.1.0",
            "platform": "linux",
            "arch": "aarch64",
            "boot_id": "boot",
            "agent_instance_id": Uuid::nil().to_string(),
            "capabilities": ["exec"]
        });
        let decoded: DeviceHello = serde_json::from_value(wire).unwrap();
        assert!(decoded.metadata.is_empty());
        assert_eq!(
            decoded,
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
        );
    }

    #[test]
    fn signed_auth_message_fields_are_length_prefixed() {
        let mut h = hello();
        h.device_id = "cd".into();
        let epoch = Uuid::parse_str("00000000-0000-4000-8000-000000000003").unwrap();
        let payload = signed_auth_message("ab", epoch, &h);
        assert!(payload.starts_with(b"infra-v1-device-auth\0"));
        assert_eq!(&payload[21..25], &2u32.to_be_bytes()[..]);
        assert_eq!(&payload[25..27], b"ab");
        assert_eq!(&payload[27..31], &16u32.to_be_bytes()[..]);
        assert_eq!(&payload[31..47], &epoch.as_bytes()[..]);
        assert_eq!(&payload[47..51], &2u32.to_be_bytes()[..]);
        assert_eq!(&payload[51..53], &PROTOCOL_VERSION.to_be_bytes()[..]);
        let mut shifted = hello();
        shifted.device_id = "bcd".into();
        assert_ne!(payload, signed_auth_message("a", epoch, &shifted));
    }
}
