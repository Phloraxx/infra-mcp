//! Versioned wire types shared by the Infra gateway and device agent.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 1;
pub const DEFAULT_CONTROL_FRAME_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceHello {
    pub protocol: u16,
    pub token: String,
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
pub struct HelloAck {
    pub protocol: u16,
    pub connection_generation: u64,
    pub gateway_epoch: Uuid,
    pub control_frame_limit: usize,
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
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum AgentToGateway {
    Hello(DeviceHello),
    Response(AgentResponse),
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum GatewayToAgent {
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

    #[test]
    fn hello_round_trip() {
        let hello = AgentToGateway::Hello(DeviceHello {
            protocol: PROTOCOL_VERSION,
            token: "test".into(),
            device_id: "oracle".into(),
            device_name: "Oracle".into(),
            agent_version: "0.1.0".into(),
            platform: "linux".into(),
            arch: "aarch64".into(),
            boot_id: "boot".into(),
            agent_instance_id: Uuid::nil(),
            capabilities: vec!["exec".into()],
            metadata: BTreeMap::new(),
        });
        let json = serde_json::to_string(&hello).unwrap();
        let decoded: AgentToGateway = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, hello);
    }
}
