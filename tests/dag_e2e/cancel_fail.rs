//! D3 — cancel mid-run and step failure leave exactly the promised
//! traces: cancelling a live run flips it to `cancelling`, the node
//! drains the in-flight binary step, marks the not-yet-started step
//! unfinished (`run cancelled`), folds the execution to `cancelled` and
//! keeps every step's meta.json; an agent step whose LLM stub answers
//! with a non-retryable status fails the run (`error`).

use crate::fixtures::{publish, SPIN_C};
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::{LlmStub, Script};
use serde_json::{json, Value};

/// Read a node artifact and parse it as JSON.
fn read_json(path: &std::path::Path) -> Value {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

#[test]
fn cancel_mid_run_and_agent_step_failure() {
    // One scripted reply: the fail scenario's agent step. 400 is NOT in
    // the retryable set (408|425|429|5xx), so the step fails on attempt 1.
    let stub = LlmStub::spawn(vec![Script::Fail(400, "bad prompt".into())]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_native(tmp.path(), stub.port(), json!({}), "dag-cancel-node");

    // Spin module: sleeps 3s then echoes — long enough to cancel mid-run.
    publish(&fleet, "spin", SPIN_C);
    let spec = json!({
        "name": "e2e-spin",
        "steps": [
            {"name": "spin", "kind": {"type":"binary","resource":"spin"}},
            {"name": "after", "depends_on": ["spin"],
             "kind": {"type": "agent", "prompt": "never runs"}},
        ],
    });
    let (status, body) = fleet.http("POST", "/api/dag/defs", &json!({"spec": spec}));
    assert_eq!(status, 200, "save spin def: {body}");
    let run = "dag-cancel-run";
    let (status, body) = fleet.http(
        "POST",
        "/api/dag/defs/e2e-spin/dispatch",
        &json!({"id": run}),
    );
    assert_eq!(status, 202, "dispatch {run}: {body}");

    // Cancel while `spin` is in flight: the run flips to `cancelling`
    // (pending would collapse straight to cancelled).
    fleet.wait_status(run, "spin step running", 30, |doc| {
        doc["dag_steps"]["running"] == 1
    });
    let (status, body) = fleet.http("POST", &format!("/api/dag/runs/{run}/cancel"), &json!({}));
    assert_eq!(status, 200, "cancel: {body}");
    assert_eq!(body["status"], "cancelling");

    // The node drains the in-flight step, marks `after` unfinished and
    // folds the execution to cancelled (cancelled > error > done).
    let doc = fleet.wait_terminal(run);
    assert_eq!(doc["execution"]["status"], "cancelled", "fold: {doc}");
    let (status, progress) =
        fleet.http("GET", &format!("/api/dag/runs/{run}/progress"), &json!({}));
    assert_eq!(status, 200);
    assert_eq!(progress["execution_status"], "cancelled");
    assert_eq!(progress["total"], 2);
    assert_eq!(
        progress["cancelled"], 2,
        "in-flight + never-started: {progress}"
    );
    assert_eq!(progress["done"], 0);
    assert_eq!(progress["error"], 0);

    // Both steps keep a meta.json (the LOCKED step-io contract survives
    // cancellation); the never-started step's error names the reason.
    let spin_meta = read_json(&fleet.run_root(run).join("spin/meta.json"));
    assert_eq!(spin_meta["step"], "spin");
    assert_eq!(spin_meta["outcome"], "cancelled");
    let after_meta = read_json(&fleet.run_root(run).join("after/meta.json"));
    assert_eq!(after_meta["outcome"], "cancelled");
    assert_eq!(after_meta["error"], "run cancelled");
    // The drained binary step produced no stdout (killed mid-sleep).
    let spin_out = std::fs::read_to_string(fleet.run_root(run).join("spin/output.txt")).unwrap();
    assert_eq!(spin_out.trim(), "");

    // Agent step with a non-retryable stub reply: the run errors out.
    let fail_spec = json!({
        "name": "e2e-agent-fail",
        "steps": [{"name": "agent", "kind": {"type": "agent", "prompt": "fail me"}}],
    });
    let (status, body) = fleet.http("POST", "/api/dag/defs", &json!({"spec": fail_spec}));
    assert_eq!(status, 200, "save fail def: {body}");
    let fail_run = "dag-fail-run";
    let (status, body) = fleet.http(
        "POST",
        "/api/dag/defs/e2e-agent-fail/dispatch",
        &json!({"id": fail_run}),
    );
    assert_eq!(status, 202, "dispatch fail: {body}");
    let doc = fleet.wait_terminal(fail_run);
    assert_eq!(
        doc["execution"]["status"], "error",
        "agent step failure: {doc}"
    );
    let (status, progress) = fleet.http(
        "GET",
        &format!("/api/dag/runs/{fail_run}/progress"),
        &json!({}),
    );
    assert_eq!(status, 200);
    assert_eq!(progress["error"], 1);
    assert!(!progress["steps"][0]["error"]
        .as_str()
        .unwrap_or_default()
        .is_empty());
    let meta = read_json(&fleet.run_root(fail_run).join("agent/meta.json"));
    assert_eq!(meta["outcome"], "error");
    assert!(!meta["error"].as_str().unwrap_or_default().is_empty());
    // The failure surfaced on the run event stream, ending run_finished.
    assert_eq!(doc["result"]["status"], "error");

    let requests = stub.wait_for_requests(1);
    assert!(requests[0].contains("fail me"), "prompt: {}", requests[0]);
}

#[test]
fn runc_preflight_rejects_missing_or_incomplete_rootfs() {
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_native(tmp.path(), stub.port(), json!({}), "dag-preflight-node");
    publish(&fleet, "spin", SPIN_C);
    let (status, body) = fleet.http("POST", "/api/dag/defs", &json!({"spec": {
        "name":"e2e-preflight", "steps":[{"name":"run", "kind":{"type":"binary", "resource":"spin"}}]
    }}));
    assert_eq!(status, 200, "{body}");
    let config_file = fleet.workdir.join("node-work/.opencoder/config.json");
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config_file).unwrap()).unwrap();
    let incomplete = tmp.path().join("incomplete-rootfs");
    for (index, path) in [tmp.path().join("missing-rootfs"), incomplete.clone()]
        .into_iter()
        .enumerate()
    {
        config["dag"]["rootfs_dir"] = json!(path);
        std::fs::write(&config_file, config.to_string()).unwrap();
        let (status, body) = fleet.http(
            "POST",
            "/api/dag/defs/e2e-preflight/dispatch",
            &json!({"id":format!("dag-preflight-{index}")}),
        );
        assert_eq!(status, 400, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("preflight"),
            "{body}"
        );
        std::fs::create_dir_all(&incomplete).unwrap();
    }
    assert_eq!(stub.request_count(), 0);
}
