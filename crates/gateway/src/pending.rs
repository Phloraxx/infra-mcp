//! Short synchronous bookkeeping so dropping a request also removes its waiter.
//! No mutex is held across I/O or an await.
use infra_protocol::AgentResponse;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Clone, Default)]
pub(crate) struct PendingRequests(Arc<Mutex<HashMap<Uuid, Entry>>>);

struct Entry {
    device: String,
    generation: u64,
    reply: oneshot::Sender<AgentResponse>,
}

pub(crate) struct PendingRequest {
    id: Uuid,
    registry: PendingRequests,
}

impl PendingRequests {
    pub(crate) fn register(
        &self,
        id: Uuid,
        device: String,
        generation: u64,
    ) -> (PendingRequest, oneshot::Receiver<AgentResponse>) {
        let (reply, receiver) = oneshot::channel();
        self.0.lock().expect("pending registry poisoned").insert(
            id,
            Entry {
                device,
                generation,
                reply,
            },
        );
        (
            PendingRequest {
                id,
                registry: self.clone(),
            },
            receiver,
        )
    }

    pub(crate) fn complete(&self, device: &str, generation: u64, response: AgentResponse) {
        let reply = {
            let mut entries = self.0.lock().expect("pending registry poisoned");
            if !entries
                .get(&response.id)
                .is_some_and(|entry| entry.device == device && entry.generation == generation)
            {
                return;
            }
            entries.remove(&response.id).map(|entry| entry.reply)
        };
        if let Some(reply) = reply {
            let _ = reply.send(response);
        }
    }

    pub(crate) fn disconnect(&self, device: &str, generation: u64) {
        self.0
            .lock()
            .expect("pending registry poisoned")
            .retain(|_, entry| entry.device != device || entry.generation != generation);
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.lock().expect("pending registry poisoned").is_empty()
    }
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        self.registry
            .0
            .lock()
            .expect("pending registry poisoned")
            .remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use infra_protocol::OperationResult;

    fn response(id: Uuid) -> AgentResponse {
        AgentResponse {
            id,
            result: OperationResult::Ok {
                body: serde_json::json!({}),
            },
        }
    }

    #[tokio::test]
    async fn only_the_originating_device_and_generation_can_complete_a_request() {
        let pending = PendingRequests::default();
        let id = Uuid::new_v4();
        let (_guard, mut receiver) = pending.register(id, "test-device".into(), 2);
        pending.complete("another-device", 2, response(id));
        pending.complete("test-device", 1, response(id));
        assert!(receiver.try_recv().is_err());
        assert!(!pending.is_empty());
        pending.complete("test-device", 2, response(id));
        assert_eq!(receiver.await.unwrap().id, id);
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn disconnect_only_releases_waiters_for_that_connection() {
        let pending = PendingRequests::default();
        let (_old, old) = pending.register(Uuid::new_v4(), "test-device".into(), 1);
        let (_new, mut new) = pending.register(Uuid::new_v4(), "test-device".into(), 2);
        pending.disconnect("test-device", 1);
        assert!(old.await.is_err());
        assert!(new.try_recv().is_err());
        assert!(!pending.is_empty());
    }

    #[tokio::test]
    async fn dropping_request_synchronously_removes_waiter() {
        let pending = PendingRequests::default();
        let (guard, receiver) = pending.register(Uuid::new_v4(), "test-device".into(), 1);
        drop(guard);
        assert!(pending.is_empty());
        assert!(receiver.await.is_err());
    }
}
