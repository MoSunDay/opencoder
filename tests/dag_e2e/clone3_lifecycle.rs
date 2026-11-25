//! Native sandbox lifecycle, end to end against the real fleet: the pinned
//! backend needs no OCI runtime and is never silently replaced by it, and a
//! node restart tears the orphaned sandbox down through its lease, marks the
//! run `interrupted` and lets an explicit resume reuse the same run root.

use std::process::Command;

use serde_json::{json, Value};

use crate::clone3_fixtures::{
    assert_released, backend_ready, clone3_only, path_without_tools, read_json, wait_for_live,
    wait_gone, LATE_C,
};
use crate::fixtures::{publish, stdout_source};
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;

fn save(fleet: &Fleet, spec: Value) {
    let (status, body) = fleet.http("POST", "/api/dag/defs", &json!({"spec": spec}));
    assert_eq!(status, 200, "save def: {body}");
}

fn dispatch_raw(fleet: &Fleet, definition: &str, run: &str) -> (u16, Value) {
    fleet.http(
        "POST",
        &format!("/api/dag/defs/{definition}/dispatch"),
        &json!({"id": run}),
    )
}

fn step_text(run_root: &std::path::Path, step: &str, file: &str) -> String {
    std::fs::read_to_string(run_root.join(step).join(file))
        .unwrap_or_else(|error| panic!("read {step}/{file}: {error}"))
}

/// Rewrite the node's `dag.sandbox` in place: the node reads its config per
/// dispatch, so one fleet can be re-pinned between runs.
fn pin_sandbox(fleet: &Fleet, sandbox: &str) {
    let file = fleet.workdir.join("node-work/.opencoder/config.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    config["dag"]["sandbox"] = json!(sandbox);
    std::fs::write(&file, config.to_string()).unwrap();
}

#[test]
fn pinned_native_backend_needs_no_runc_and_is_never_replaced_by_it() {
    if !clone3_only("backend pinning") {
        return;
    }
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    // The node cannot even resolve an OCI runtime.
    let path = path_without_tools(&tmp.path().join("toolless"));
    let probe = Command::new("/bin/sh")
        .args(["-c", "command -v runc || true"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(probe.stdout.is_empty(), "fixture PATH still finds runc");
    let fleet = Fleet::spawn_native_with(
        tmp.path(),
        stub.port(),
        json!({}),
        "pinned-native-node",
        vec![("PATH".into(), path)],
    );

    publish(&fleet, "hello", &stdout_source("native backend"));
    let definition = "e2e-pinned-native";
    save(
        &fleet,
        json!({"name": definition, "steps": [
            {"name": "run", "kind": {"type":"binary", "resource":"hello"}},
        ]}),
    );
    let cases: [(usize, &str, bool); 3] =
        [(0, "clone3", true), (1, "runc", false), (2, "clone3", true)];
    for (index, sandbox, accepted) in cases {
        pin_sandbox(&fleet, sandbox);
        let run = format!("dag-pinned-native-{index}");
        let (status, body) = dispatch_raw(&fleet, definition, &run);
        if !accepted {
            // Pinning the OCI backend on a node without it fails in
            // preflight, naming the requirement: no fallback to clone3.
            assert_eq!(status, 400, "dispatch {run}: {body}");
            let error = body["error"].as_str().unwrap_or_default();
            assert!(
                error.contains("preflight") && error.contains("runc"),
                "{error}"
            );
            continue;
        }
        assert_eq!(status, 202, "dispatch {run}: {body}");
        let doc = fleet.wait_terminal(&run);
        assert_eq!(doc["execution"]["status"], "done", "inspect: {doc}");
        let run_root = fleet.run_root(&run);
        assert!(step_text(&run_root, "run", "output.txt").contains("native backend"));
        let receipt = read_json(&run_root.join("container.json"));
        assert_eq!(receipt["backend"], "clone3", "{receipt}");
        // Nothing of the OCI runtime is materialised for a native run.
        assert!(
            !run_root.join("runc-state").exists(),
            "OCI state under a native run"
        );
    }
    assert_eq!(stub.request_count(), 0);
}

#[test]
fn node_restart_interrupts_the_run_and_resume_reuses_it() {
    if !backend_ready() {
        return;
    }
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let mut fleet = Fleet::spawn_native(tmp.path(), stub.port(), json!({}), "sandbox-restart-node");
    publish(&fleet, "late", LATE_C);
    publish(&fleet, "after", &stdout_source("after"));
    let definition = "e2e-sandbox-restart";
    let run = "dag-e2e-sandbox-restart-1";
    save(
        &fleet,
        json!({"name": definition, "steps": [
            {"name": "late", "kind": {"type":"binary", "resource":"late"}},
            {"name": "after", "depends_on": ["late"],
             "kind": {"type":"binary", "resource":"after"}},
        ]}),
    );
    let (status, body) = dispatch_raw(&fleet, definition, run);
    assert_eq!(status, 202, "dispatch {run}: {body}");
    fleet.wait_status(run, "late step running", 30, |doc| {
        doc["dag_steps"]["running"] == 1
    });
    let run_root = fleet.run_root(run);
    let live = wait_for_live(&run_root);
    let node_id = fleet.node_id();

    // The node dies. The native sandbox is leased to it: the orphaned
    // launcher loses the lease and tears the sandbox down on its own.
    fleet.kill_agent();
    if live.launcher.is_some() {
        wait_gone(&live, "lease teardown after node death");
    }

    // The node comes back: startup recovery releases the sandbox and marks
    // the in-flight run `interrupted`, awaiting an explicit resume.
    fleet.respawn_agent();
    let doc = fleet.wait_status(run, "interrupted status", 60, |doc| {
        doc["execution"]["status"] == "interrupted"
    });
    assert_eq!(
        doc["error"], "node process stopped; explicit resume required",
        "{doc}"
    );
    assert_eq!(doc["execution"]["node_id"], node_id, "{doc}");
    assert_eq!(doc["dag_steps"]["interrupted"], 1, "{}", doc["dag_steps"]);
    assert_released(&run_root, Some(&live), fleet.agent_pid());

    // Resume re-runs the interrupted step on the same run root and the
    // dependent step follows.
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/executions/{run}/commands"),
        &json!({"action": "resume", "input": {}}),
    );
    assert!(matches!(status, 200 | 409), "resume {run}: {status} {body}");
    let doc = fleet.wait_terminal(run);
    assert_eq!(doc["execution"]["status"], "done", "inspect: {doc}");
    assert_eq!(doc["result"]["status"], "done", "{}", doc["result"]);
    assert_eq!(doc["dag_context"]["container_id"], format!("dag-run-{run}"));
    assert!(step_text(&run_root, "late", "output.txt").contains("late"));
    assert!(step_text(&run_root, "after", "output.txt").contains("after"));
    let meta = read_json(&run_root.join("late/meta.json"));
    assert_eq!(meta["outcome"], "done", "{meta}");
    assert_released(
        &run_root,
        crate::clone3_fixtures::recorded(&run_root).as_ref(),
        fleet.agent_pid(),
    );
    assert_eq!(stub.request_count(), 0);
}
