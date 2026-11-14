//! Dashboard event stream (SSE) observer.
//!
//! `sessions_changed` is the event that drives dashboard refresh churn, so the
//! harness counts it directly instead of inferring it from logs.

use serde::Deserialize;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DashboardEvent {
    pub kind: String,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub at_unix_ms: u64,
}

pub struct EventStream {
    events: Arc<Mutex<Vec<DashboardEvent>>>,
}

impl EventStream {
    pub async fn start(base: &str, token: &str) -> Result<Self, String> {
        let response = reqwest::Client::new()
            .get(format!("{}/api/events", base.trim_end_matches('/')))
            .bearer_auth(token)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .map_err(|error| format!("event stream request failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("event stream HTTP {}", response.status()));
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        tokio::spawn(async move {
            let mut response = response;
            let mut buffer: Vec<u8> = Vec::new();
            while let Ok(Some(chunk)) = response.chunk().await {
                buffer.extend_from_slice(&chunk);
                while let Some(index) = buffer.windows(2).position(|pair| pair == b"\n\n") {
                    let block: Vec<u8> = buffer.drain(..index + 2).collect();
                    for line in block.split(|byte| *byte == b'\n') {
                        let Some(data) = line.strip_prefix(b"data: ") else {
                            continue;
                        };
                        if let Ok(event) = serde_json::from_slice::<DashboardEvent>(data) {
                            sink.lock().expect("event sink poisoned").push(event);
                        }
                    }
                }
            }
        });
        Ok(Self { events })
    }

    /// Remove and return everything observed so far.
    pub fn take(&self) -> Vec<DashboardEvent> {
        std::mem::take(&mut *self.events.lock().expect("event sink poisoned"))
    }

    pub fn drain(&self) {
        self.events.lock().expect("event sink poisoned").clear();
    }

    pub fn count(&self, kind: &str) -> usize {
        self.events
            .lock()
            .expect("event sink poisoned")
            .iter()
            .filter(|event| event.kind == kind)
            .count()
    }

    pub fn count_for(&self, kind: &str, device_id: &str) -> usize {
        self.events
            .lock()
            .expect("event sink poisoned")
            .iter()
            .filter(|event| event.kind == kind && event.device_id.as_deref() == Some(device_id))
            .count()
    }
}
