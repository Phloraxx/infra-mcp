//! Non-secret status receipts; disk I/O stays off the socket receive loop.
use infra_local_state::{ConnectionProof, write_connection_proof};
use infra_protocol::{ConnectionProbe, ConnectionProbeReply, PROTOCOL_VERSION};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::watch;

#[derive(Default)]
pub struct Exchange(Option<(ConnectionProbeReply, Instant)>);
impl Exchange {
    pub fn reply(
        &mut self,
        probe: ConnectionProbe,
        generation: u64,
    ) -> Option<ConnectionProbeReply> {
        if probe.generation != generation {
            return None;
        }
        let reply = ConnectionProbeReply {
            probe,
            pid: std::process::id(),
            protocol: PROTOCOL_VERSION,
        };
        self.0 = Some((reply.clone(), Instant::now()));
        Some(reply)
    }
    pub fn confirmed(&mut self, reply: &ConnectionProbeReply) -> bool {
        let valid = self.0.as_ref().is_some_and(|(expected, issued)| {
            *expected == *reply && issued.elapsed() <= Duration::from_secs(10)
        });
        if valid {
            self.0 = None;
        }
        valid
    }
}

/// One writer for the controller lifetime, one coalesced pending receipt. An
/// old/slow write retains its original verification time and session identity.
pub fn proof_writer(state_dir: PathBuf) -> watch::Sender<Option<ConnectionProof>> {
    let (tx, mut rx) = watch::channel::<Option<ConnectionProof>>(None);
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let proof = rx.borrow_and_update().clone();
            if let Some(proof) = proof {
                let dir = state_dir.clone();
                match tokio::task::spawn_blocking(move || write_connection_proof(&dir, &proof))
                    .await
                {
                    Ok(Ok(())) => {}
                    _ => tracing::warn!(
                        "could not persist connection verification; CLI will report unverified"
                    ),
                }
            }
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    #[test]
    fn confirmation_must_match_the_current_probe_and_status() {
        let mut exchange = Exchange::default();
        let probe = ConnectionProbe {
            nonce: Uuid::new_v4(),
            generation: 7,
        };
        assert!(exchange.reply(probe.clone(), 8).is_none());
        let reply = exchange.reply(probe.clone(), 7).unwrap();
        let mut wrong = reply.clone();
        wrong.pid += 1;
        assert!(!exchange.confirmed(&wrong));
        wrong = reply.clone();
        wrong.protocol = 0;
        assert!(!exchange.confirmed(&wrong));
        wrong = reply.clone();
        wrong.probe.nonce = Uuid::new_v4();
        assert!(!exchange.confirmed(&wrong));
        exchange.0.as_mut().unwrap().1 = Instant::now() - Duration::from_secs(11);
        assert!(!exchange.confirmed(&reply));
        exchange.reply(probe, 7);
        assert!(exchange.confirmed(&reply));
        assert!(!exchange.confirmed(&reply));
    }
}
