//! One outstanding, generation-bound status challenge per signed connection.
use infra_protocol::{ConnectionProbe, ConnectionProbeReply, PROTOCOL_VERSION};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const PROBE_LIFETIME: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct ProbeState(Option<(ConnectionProbe, Instant)>);
impl ProbeState {
    pub fn challenge(&mut self, generation: u64) -> ConnectionProbe {
        let probe = ConnectionProbe {
            nonce: Uuid::new_v4(),
            generation,
        };
        self.0 = Some((probe.clone(), Instant::now()));
        probe
    }
    pub fn accept(&mut self, reply: &ConnectionProbeReply) -> bool {
        let valid = self.0.as_ref().is_some_and(|(probe, issued)| {
            *probe == reply.probe
                && issued.elapsed() <= PROBE_LIFETIME
                && reply.pid != 0
                && reply.protocol == PROTOCOL_VERSION
        });
        if valid {
            self.0 = None;
        }
        valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_rejects_wrong_generation_nonce_protocol_expiry_and_replay() {
        let mut state = ProbeState::default();
        let probe = state.challenge(7);
        let reply = ConnectionProbeReply {
            probe: probe.clone(),
            pid: 42,
            protocol: PROTOCOL_VERSION,
        };
        let mut wrong = reply.clone();
        wrong.probe.generation = 8;
        assert!(!state.accept(&wrong));
        wrong = reply.clone();
        wrong.probe.nonce = Uuid::new_v4();
        assert!(!state.accept(&wrong));
        wrong = reply.clone();
        wrong.protocol = 0;
        assert!(!state.accept(&wrong));
        wrong = reply.clone();
        wrong.pid = 0;
        assert!(!state.accept(&wrong));
        state.0.as_mut().unwrap().1 = Instant::now() - PROBE_LIFETIME - Duration::from_millis(1);
        assert!(!state.accept(&reply));
        state.0 = Some((probe, Instant::now()));
        assert!(state.accept(&reply));
        assert!(!state.accept(&reply));
    }
}
