//! Real-binary fault-injection harness for the Infra gateway and agent.
//!
//! Every fixture is hermetic: the workspace binaries are spawned from `target/`,
//! each fixture owns a temp directory, and all traffic stays on 127.0.0.1. The
//! only failures are the ones [`FaultProxy`] injects between the agent and the
//! gateway, so a scenario can cut, delay, split or blackhole a real socket
//! without touching a live service.
//!
//! The harness is Unix-only: it drives POSIX shell commands and PTYs through the
//! real agent. Windows acceptance is a separate gate.

mod events;
mod fixture;
mod mcp;
mod proxy;
mod store;

pub use events::{DashboardEvent, EventStream};
pub use fixture::{Agent, Fixture, Gateway, HEARTBEAT_TIMEOUT_SECS};
pub use mcp::McpClient;
pub use proxy::{FaultProxy, Faults};
pub use store::{AuditRow, DeviceRow, StoreProbe};
