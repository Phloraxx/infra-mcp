use infra_local_state::{
    ConnectionProof, LocalConfig, MAX_CONNECTION_PROOF_AGE_MS, RuntimeStatus, read_connection_proof,
};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct ConnectionStatus {
    pub verified: bool,
    pub code: &'static str,
    pub detail: &'static str,
    pub verified_unix_ms: Option<u64>,
    pub probe_started_unix_ms: Option<u64>,
    pub age_ms: Option<u64>,
}

pub fn inspect(
    dir: &Path,
    config: Option<&LocalConfig>,
    runtime: Option<&RuntimeStatus>,
    alive: bool,
    now: u64,
) -> ConnectionStatus {
    match read_connection_proof(dir) {
        Ok(proof) => evaluate(config, runtime, proof.as_ref(), alive, now),
        Err(_) => problem(
            "connection_proof_unreadable",
            "Connection receipt is unreadable. Run doctor; update the gateway and agent if verification stays unavailable.",
        ),
    }
}

fn problem(code: &'static str, detail: &'static str) -> ConnectionStatus {
    ConnectionStatus {
        verified: false,
        code,
        detail,
        verified_unix_ms: None,
        probe_started_unix_ms: None,
        age_ms: None,
    }
}

fn evaluate(
    config: Option<&LocalConfig>,
    runtime: Option<&RuntimeStatus>,
    proof: Option<&ConnectionProof>,
    alive: bool,
    now: u64,
) -> ConnectionStatus {
    let Some(runtime) = runtime.filter(|_| alive) else {
        return problem(
            "agent_not_running",
            "No live agent process is confirmed. Run infra doctor and repair startup if needed.",
        );
    };
    if runtime.state != "online" {
        return problem(
            "connection_offline",
            "The agent is not reporting an online connection. Check connectivity before repair.",
        );
    }
    let (Some(config), Some(proof)) = (config, proof) else {
        return problem(
            "connection_unverified",
            "No signed connection receipt is available. Wait for a roundtrip; update both gateway and agent if this persists.",
        );
    };
    if runtime.pid != proof.pid
        || runtime.device_id.as_deref() != Some(proof.device_id.as_str())
        || runtime.connection_generation != Some(proof.connection_generation)
        || runtime.gateway_epoch != Some(proof.gateway_epoch)
        || runtime.agent_instance_id != Some(proof.agent_instance_id)
        || infra_local_state::gateway_fingerprint(&config.gateway_url) != proof.gateway_fingerprint
    {
        return problem(
            "connection_proof_mismatch",
            "The receipt belongs to a different process, gateway or connection. Wait for fresh verification.",
        );
    }
    if proof.probe_started_unix_ms > proof.verified_unix_ms {
        return problem(
            "connection_clock_invalid",
            "Connection receipt predates its probe. Check the system clock and wait for fresh verification.",
        );
    }
    let Some(age) = now.checked_sub(proof.verified_unix_ms) else {
        return problem(
            "connection_clock_invalid",
            "Connection receipt is in the future. Check the system clock and wait for fresh verification.",
        );
    };
    let mut result = if age <= MAX_CONNECTION_PROOF_AGE_MS {
        ConnectionStatus {
            verified: true,
            code: "connection_verified",
            detail: "Signed status roundtrip verified within the last 30 seconds.",
            verified_unix_ms: None,
            probe_started_unix_ms: None,
            age_ms: None,
        }
    } else {
        problem(
            "connection_proof_stale",
            "The last signed status roundtrip is stale. Check gateway connectivity; cached online state is not current verification.",
        )
    };
    result.verified_unix_ms = Some(proof.verified_unix_ms);
    result.probe_started_unix_ms = Some(proof.probe_started_unix_ms);
    result.age_ms = Some(age);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (LocalConfig, RuntimeStatus, ConnectionProof) {
        let config = LocalConfig::new("ws://127.0.0.1:12345/agent".into(), None).unwrap();
        // IDs are arbitrary fixture values, not enrolled devices.
        let epoch = serde_json::from_str("\"11111111-1111-4111-8111-111111111111\"").unwrap();
        let instance = serde_json::from_str("\"22222222-2222-4222-8222-222222222222\"").unwrap();
        let mut runtime = RuntimeStatus::new("online");
        runtime.device_id = Some("fixture".into());
        runtime.connection_generation = Some(7);
        runtime.gateway_epoch = Some(epoch);
        runtime.agent_instance_id = Some(instance);
        let proof = ConnectionProof {
            version: 1,
            pid: runtime.pid,
            device_id: "fixture".into(),
            gateway_fingerprint: infra_local_state::gateway_fingerprint(&config.gateway_url),
            gateway_epoch: epoch,
            agent_instance_id: instance,
            connection_generation: 7,
            probe_started_unix_ms: 99_999,
            verified_unix_ms: 100_000,
        };
        (config, runtime, proof)
    }
    #[test]
    fn online_and_live_pid_alone_are_not_verified() {
        let (config, runtime, proof) = fixture();
        assert_eq!(
            evaluate(Some(&config), Some(&runtime), None, true, 100_000).code,
            "connection_unverified"
        );
        assert!(!evaluate(Some(&config), Some(&runtime), Some(&proof), false, 100_000).verified);
        let mut offline = runtime;
        offline.state = "reconnecting".into();
        assert_eq!(
            evaluate(Some(&config), Some(&offline), Some(&proof), true, 100_000).code,
            "connection_offline"
        );
    }
    #[test]
    fn receipts_are_bound_to_process_identity_gateway_epoch_instance_and_generation() {
        let (config, runtime, proof) = fixture();
        assert!(evaluate(Some(&config), Some(&runtime), Some(&proof), true, 100_001).verified);
        for field in 0..6 {
            let mut wrong = proof.clone();
            match field {
                0 => wrong.pid += 1,
                1 => wrong.device_id = "other".into(),
                2 => wrong.gateway_fingerprint.push_str("other"),
                3 => wrong.gateway_epoch = wrong.agent_instance_id,
                4 => wrong.agent_instance_id = wrong.gateway_epoch,
                _ => wrong.connection_generation += 1,
            };
            assert_eq!(
                evaluate(Some(&config), Some(&runtime), Some(&wrong), true, 100_001).code,
                "connection_proof_mismatch"
            );
        }
    }
    #[test]
    fn stale_future_and_unreadable_receipts_fail_closed() {
        let (config, runtime, proof) = fixture();
        assert_eq!(
            evaluate(Some(&config), Some(&runtime), Some(&proof), true, 130_001).code,
            "connection_proof_stale"
        );
        assert_eq!(
            evaluate(Some(&config), Some(&runtime), Some(&proof), true, 99_999).code,
            "connection_clock_invalid"
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("connection-proof.json"), b"broken").unwrap();
        assert_eq!(
            inspect(dir.path(), Some(&config), Some(&runtime), true, 100_000).code,
            "connection_proof_unreadable"
        );
    }
}
