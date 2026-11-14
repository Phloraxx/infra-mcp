//! Copy throughput and control-latency benchmark.
//!
//! Manual, not part of the default suite: it copies 1/16/100 MiB through a real
//! gateway and two real agents several times and prints p50/p95. Run it with
//!
//! ```text
//! cargo test -p infra-e2e --test bench_copy -- --ignored --nocapture
//! ```
//!
//! The same file runs unchanged against a tree without bulk frames, which is how
//! the before/after numbers in docs/BENCHMARK.md were produced: the copy tool
//! reports `transport` when the gateway knows about it, and the benchmark only
//! prints it.

#![cfg(target_os = "linux")]

use infra_e2e::{Agent, Fixture};
use serde_json::{Value, json};
use std::{
    fs,
    time::{Duration, Instant},
};

const SIZES: [(&str, usize); 3] = [
    ("1 MiB", 1 << 20),
    ("16 MiB", 16 << 20),
    ("100 MiB", 100 << 20),
];
const ITERATIONS: usize = 5;
const MAX_BYTES_PER_CALL: u64 = 64 * 1024 * 1024;

async fn enroll(gateway_base: &str, token: &str) -> String {
    let response = reqwest::Client::new()
        .post(format!("{gateway_base}/api/enrollments"))
        .bearer_auth(token)
        .json(&json!({"ttl_seconds": 300}))
        .send()
        .await
        .expect("create enrollment");
    let body: Value = response.json().await.expect("decode enrollment");
    body["code"].as_str().expect("enrollment code").to_owned()
}

async fn wait_device_named(fixture: &Fixture, name: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(result) = fixture.mcp.call("devices", json!({})).await
            && let Some(devices) = result["devices"].as_array()
            && let Some(device) = devices.iter().find(|device| {
                device["device_name"].as_str() == Some(name)
                    && device["online"].as_bool() == Some(true)
            })
            && let Some(device_id) = device["device_id"].as_str()
        {
            return device_id.to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "device {name} never became online"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn percentile(samples: &[f64], fraction: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).expect("finite samples"));
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

/// Copy one file to completion, resuming with the returned transfer IDs when a
/// call stops at its byte budget. Returns the wall time and the final body.
async fn copy_to_completion(
    fixture: &Fixture,
    from: &str,
    to: &str,
    source: &std::path::Path,
    destination: &std::path::Path,
) -> (Duration, Value) {
    let started = Instant::now();
    let mut result = fixture
        .mcp
        .call(
            "copy",
            json!({
                "source_device": from,
                "destination_device": to,
                "source_path": source,
                "destination_path": destination,
                "overwrite": true,
                "create_parents": true,
                "max_bytes": MAX_BYTES_PER_CALL,
            }),
        )
        .await
        .expect("copy call");
    let mut calls = 1;
    while result["status"] == "in_progress" && calls < 8 {
        result = fixture
            .mcp
            .call(
                "copy",
                json!({
                    "source_device": from,
                    "destination_device": to,
                    "source_path": source,
                    "destination_path": destination,
                    "source_transfer_id": result["source_transfer_id"],
                    "destination_transfer_id": result["destination_transfer_id"],
                    "max_bytes": MAX_BYTES_PER_CALL,
                }),
            )
            .await
            .expect("resume copy");
        calls += 1;
    }
    assert_eq!(result["status"], "completed", "copy failed: {result}");
    (started.elapsed(), result)
}

/// Round-trip time of a control RPC to the source device.
async fn status_rtt(fixture: &Fixture, device: &str, transfer_id: &str) -> f64 {
    let started = Instant::now();
    let result = fixture
        .mcp
        .call(
            "transfer",
            json!({
                "device": device,
                "action": "status",
                "transfer_id": transfer_id,
            }),
        )
        .await
        .expect("status call");
    assert_eq!(result["status"], "ok", "status failed: {result}");
    started.elapsed().as_secs_f64() * 1000.0
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual benchmark: cargo test -p infra-e2e --test bench_copy -- --ignored --nocapture"]
async fn copy_throughput_and_control_latency() {
    let fixture = Fixture::start().await;
    let code = enroll(&fixture.gateway.base, &fixture.gateway.token).await;
    let mut destination_agent = Agent::spawn(
        &fixture.dir.path().join("agent-bench-destination"),
        &fixture.dir.path().join("bench-destination.log"),
        &fixture.proxy.agent_url(),
        Some(&code),
        "bench-destination",
    )
    .expect("start destination agent");
    let destination_id = wait_device_named(&fixture, "bench-destination").await;
    let source_id = fixture.device_id.clone();

    println!("\ncopy benchmark: {ITERATIONS} iterations per size, 256 KiB chunks");
    println!(
        "{:<10} {:>10} {:>10} {:>10} {:>10} {:>12}",
        "size", "p50 MiB/s", "p95 MiB/s", "p50 ms", "p95 ms", "transport"
    );
    let mut transport = String::from("unknown");
    for (label, size) in SIZES {
        let source = fixture.dir.path().join(format!("bench-source-{size}.bin"));
        let data: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
        fs::write(&source, &data).expect("write benchmark source");
        let mut throughput = Vec::with_capacity(ITERATIONS);
        let mut elapsed_ms = Vec::with_capacity(ITERATIONS);
        for iteration in 0..ITERATIONS {
            let destination = fixture
                .dir
                .path()
                .join(format!("bench-destination-{size}-{iteration}.bin"));
            let (elapsed, result) =
                copy_to_completion(&fixture, &source_id, &destination_id, &source, &destination)
                    .await;
            assert_eq!(
                result["bytes_transferred"].as_u64(),
                Some(size as u64),
                "copy did not move the whole file: {result}"
            );
            if let (Some(source), Some(destination)) = (
                result["transport"]["source"].as_str(),
                result["transport"]["destination"].as_str(),
            ) {
                transport = format!("{source}/{destination}");
            }
            throughput.push(size as f64 / (1 << 20) as f64 / elapsed.as_secs_f64());
            elapsed_ms.push(elapsed.as_secs_f64() * 1000.0);
            let _ = fs::remove_file(&destination);
        }
        println!(
            "{:<10} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>12}",
            label,
            percentile(&throughput, 0.5),
            percentile(&throughput, 0.95),
            percentile(&elapsed_ms, 0.5),
            percentile(&elapsed_ms, 0.95),
            transport
        );
    }

    // Control latency: a status RPC to the source while a 16 MiB copy runs on the
    // same link, compared with the same RPC at idle.
    let source = fixture.dir.path().join("bench-latency-source.bin");
    let data: Vec<u8> = (0..16 << 20).map(|index| (index % 251) as u8).collect();
    fs::write(&source, &data).expect("write latency source");
    let begin = fixture
        .mcp
        .call(
            "transfer",
            json!({"device": source_id, "action": "download_begin", "path": source}),
        )
        .await
        .expect("download begin");
    let transfer_id = begin["body"]["transfer"]["id"]
        .as_str()
        .expect("transfer id")
        .to_owned();
    let mut idle = Vec::new();
    for _ in 0..20 {
        idle.push(status_rtt(&fixture, &source_id, &transfer_id).await);
    }
    let destination = fixture.dir.path().join("bench-latency-destination.bin");
    let mcp = fixture.mcp.clone();
    let (from, to) = (source_id.clone(), destination_id.clone());
    let (source_for_copy, destination_for_copy) = (source.clone(), destination.clone());
    let copy = tokio::spawn(async move {
        let mut result = mcp
            .call(
                "copy",
                json!({
                    "source_device": from,
                    "destination_device": to,
                    "source_path": source_for_copy,
                    "destination_path": destination_for_copy,
                    "overwrite": true,
                    "max_bytes": MAX_BYTES_PER_CALL,
                }),
            )
            .await
            .expect("copy call");
        let mut calls = 1;
        while result["status"] == "in_progress" && calls < 8 {
            result = mcp
                .call(
                    "copy",
                    json!({
                        "source_device": from,
                        "destination_device": to,
                        "source_path": source_for_copy,
                        "destination_path": destination_for_copy,
                        "source_transfer_id": result["source_transfer_id"],
                        "destination_transfer_id": result["destination_transfer_id"],
                        "max_bytes": MAX_BYTES_PER_CALL,
                    }),
                )
                .await
                .expect("resume copy");
            calls += 1;
        }
        result
    });
    let mut busy = Vec::new();
    for _ in 0..20 {
        busy.push(status_rtt(&fixture, &source_id, &transfer_id).await);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let result = copy.await.expect("copy task");
    assert_eq!(result["status"], "completed", "copy failed: {result}");
    println!(
        "\ncontrol latency (transfer status RTT): idle p50 {:.1} ms p95 {:.1} ms; during copy p50 {:.1} ms p95 {:.1} ms; ratio p50 {:.2}x p95 {:.2}x",
        percentile(&idle, 0.5),
        percentile(&idle, 0.95),
        percentile(&busy, 0.5),
        percentile(&busy, 0.95),
        percentile(&busy, 0.5) / percentile(&idle, 0.5),
        percentile(&busy, 0.95) / percentile(&idle, 0.95)
    );
    destination_agent.stop();
}
