//! Host unit tests for the pure orchestration logic (no wasm involved):
//! op evidence parsing, tail JSON, NDJSON stages, verdict assembly and
//! the atomic output.json write.

use opencoder_dag_review_tools::*;
use serde_json::json;

const SAMPLE_LOG: &str = "\
# op env_eob_deploy exit=0 duration_ms=42 timeout_secs=600 truncated=false
-- output --
deploying workspace
{\"endpoint\": \"http://127.0.0.1:18080/\"}
";

#[test]
fn op_log_header_and_output_are_separated() {
    let ev = parse_op_log(SAMPLE_LOG);
    assert_eq!(ev.exit, Some(0));
    assert_eq!(ev.killed, None);
    assert!(ev.output.starts_with("deploying workspace"));
    assert!(ev.output.contains("\"endpoint\""));
}

#[test]
fn op_log_killed_header_is_parsed() {
    let ev = parse_op_log("# op x killed(Timeout) duration_ms=9\n-- output --\npartial\n");
    assert_eq!(ev.exit, None);
    assert_eq!(ev.killed.as_deref(), Some("killed(Timeout)"));
    assert_eq!(ev.output, "partial");
}

#[test]
fn missing_marker_degrades_to_all_output() {
    let ev = parse_op_log("no markers at all");
    assert_eq!(ev.exit, None);
    assert_eq!(ev.output, "no markers at all");
}

#[test]
fn tail_json_takes_the_last_object_line() {
    let ev = parse_op_log(SAMPLE_LOG);
    let tail = tail_json(&ev.output).expect("tail json");
    assert_eq!(
        endpoint_of(&tail).as_deref(),
        Some("http://127.0.0.1:18080")
    );
    // trailing slash trimmed, host/port form also accepted
    assert_eq!(
        endpoint_of(&json!({"host": "127.0.0.1", "port": 9090})).as_deref(),
        Some("http://127.0.0.1:9090")
    );
}

#[test]
fn tail_json_rejects_noise_and_non_objects() {
    assert!(tail_json("just text\n[1,2]\n{} trailing garbage").is_none());
}

const STAGES_OUT: &str = "\
booting harness
{\"stage\": \"build\", \"status\": \"ok\"}
{\"stage\": \"unit\", \"status\": \"ok\", \"detail\": \"312 passed\"}
{\"stage\": \"e2e\", \"status\": \"fail\", \"detail\": \"smoke timeout\"}
{\"stage\": \"\", \"status\": \"ok\"}
not json at all
";

#[test]
fn ndjson_stages_parse_and_fail_closed() {
    let stages = parse_stages(STAGES_OUT);
    assert_eq!(stages.len(), 3);
    assert_eq!(stages[1].detail, "312 passed");
    assert!(!stages_pass(&stages));
    assert_eq!(failed_stages(&stages), vec!["e2e".to_string()]);
}

#[test]
fn no_stages_means_failed() {
    assert!(!stages_pass(&[]));
    let ok = parse_stages("{\"stage\": \"build\", \"status\": \"ok\"}\n");
    assert!(stages_pass(&ok));
}

#[test]
fn deploy_output_shapes_up_and_down() {
    let up = deploy_output(Some(0), None, "http://h:1", 200, "env_eob_deploy");
    assert_eq!(up["status"], "up");
    assert_eq!(up["evidence"], "ops/env_eob_deploy.log");
    let down = deploy_output(Some(0), None, "http://h:1", PROBE_ERR_UNREACHABLE, "op");
    assert_eq!(down["status"], "down");
    let killed = deploy_output(Some(1), Some("killed(Timeout)"), "http://h:1", 200, "op");
    assert_eq!(killed["status"], "down");
    assert_eq!(killed["op_killed"], "killed(Timeout)");
}

#[test]
fn harness_output_gates_on_op_and_stages() {
    let stages = parse_stages("{\"stage\": \"build\", \"status\": \"ok\"}\n");
    let passed = harness_output(Some(0), None, &stages, "harness_run_full");
    assert_eq!(passed["status"], "passed");
    assert_eq!(passed["stages"], 1);
    // op itself failed → failed even with an all-ok stage report
    let bad = harness_output(Some(2), None, &stages, "harness_run_full");
    assert_eq!(bad["status"], "failed");
}

#[test]
fn output_json_is_written_atomically() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("env-eob-up");
    std::fs::create_dir_all(&dir).unwrap();
    let out = deploy_output(Some(0), None, "http://h:1", 204, "env_eob_deploy");
    write_output_json(&dir, &out).unwrap();
    let read: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("output.json")).unwrap()).unwrap();
    assert_eq!(read["status"], "up");
    assert!(!dir.join("output.json.tmp").exists());
}

#[test]
fn step_dir_env_trims_the_runtime_trailing_slash() {
    // default (unset) is empty → trimmed stays empty; the bins run only
    // under the runtime, which always sets it.
    std::env::remove_var("OPENCODER_STEP_DIR");
    assert_eq!(step_dir_from_env(), std::path::PathBuf::from(""));
}
