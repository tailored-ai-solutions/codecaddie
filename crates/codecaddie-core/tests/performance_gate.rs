use codecaddie_core::protocol::{
    CoreRequest, CoreResponse, PROTOCOL_VERSION, read_frame, write_frame,
};
use serde::Deserialize;
use std::{
    fs,
    io::{BufReader, BufWriter},
    process::{Command, Stdio},
    time::Instant,
};

const REQUEST_COUNT: usize = 120;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReliabilityPolicy {
    schema_version: u32,
    performance: PerformanceBudgets,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PerformanceBudgets {
    first_saved_report_p95_seconds: u64,
    provider_execution_p95_seconds: u64,
    core_request_p95_milliseconds: u128,
    minimum_core_requests_per_minute: u128,
    maximum_repository_files: u64,
    maximum_repository_bytes: u64,
    maximum_reports_per_workspace: u64,
}

fn policy() -> ReliabilityPolicy {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../config/reliability-gates.json");
    serde_json::from_slice(&fs::read(path).expect("reliability policy should be readable"))
        .expect("reliability policy should satisfy its schema")
}

#[test]
fn performance_gate_keeps_the_framed_core_within_latency_and_throughput_budgets() {
    let policy = policy();
    assert_eq!(policy.schema_version, 1);
    let mut child = Command::new(env!("CARGO_BIN_EXE_codecaddie-core"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("core process should start");
    let mut input = BufWriter::new(child.stdin.take().expect("core stdin should be piped"));
    let mut output = BufReader::new(child.stdout.take().expect("core stdout should be piped"));
    let suite_started = Instant::now();
    let mut latencies = Vec::with_capacity(REQUEST_COUNT);

    for index in 0..REQUEST_COUNT {
        let request = CoreRequest {
            id: format!("performance-{index}"),
            protocol_version: PROTOCOL_VERSION,
            workspace_id: None,
            method: "system.ping".into(),
            params: Default::default(),
        };
        let started = Instant::now();
        write_frame(&mut input, &request).expect("performance request should be framed");
        let response = read_frame::<CoreResponse>(&mut output)
            .expect("performance response should be framed")
            .expect("core should answer every performance request");
        assert!(response.ok, "system.ping must remain successful");
        assert_eq!(response.id, request.id);
        latencies.push(started.elapsed().as_millis());
    }

    drop(input);
    assert!(child.wait().expect("core process should exit").success());
    latencies.sort_unstable();
    let p95_index = ((latencies.len() * 95).div_ceil(100)).saturating_sub(1);
    let p95 = latencies[p95_index];
    let elapsed_millis = suite_started.elapsed().as_millis().max(1);
    let requests_per_minute = REQUEST_COUNT as u128 * 60_000 / elapsed_millis;

    assert!(
        p95 <= policy.performance.core_request_p95_milliseconds,
        "core request p95 {p95}ms exceeded {}ms",
        policy.performance.core_request_p95_milliseconds
    );
    assert!(
        requests_per_minute >= policy.performance.minimum_core_requests_per_minute,
        "core throughput {requests_per_minute}/minute fell below {}/minute",
        policy.performance.minimum_core_requests_per_minute
    );
}

#[test]
fn performance_policy_keeps_repository_and_history_capacity_explicit() {
    let budgets = policy().performance;
    assert!(budgets.first_saved_report_p95_seconds <= 10 * 60);
    assert!(budgets.provider_execution_p95_seconds <= 8 * 60);
    assert!(budgets.maximum_repository_files >= 100_000);
    assert!(budgets.maximum_repository_bytes >= 2 * 1024 * 1024 * 1024);
    assert!(budgets.maximum_reports_per_workspace >= 1_000);
}
