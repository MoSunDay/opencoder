//! D1 — the full DAG face: save a spec through `POST /api/dag/defs`,
//! dispatch it, watch one wasm step and one agent step execute on the real
//! node, then verify the artifacts, the progress/step projections, the run
//! event stream and the control-plane CLI all agree.

use crate::fixtures::{publish, stdout_module_wat};
use crate::support::fleet_proc::{Fleet, TOKEN};
use crate::support::http_util::sse_read;
use crate::support::llm_stub::{LlmStub, EXTRA_REPLY};
use crate::support::{sibling_bin, CLI_BIN};
use serde_json::{json, Value};
use std::process::Command;

const ECHO_TEXT: &str = "dag-e2e-echo-v1";
const AGENT_TEXT: &str = "dag-e2e-agent-reply";
const DEF: &str = "e2e-flow";
const RUN: &str = "dag-e2e-flow-1";

fn spec() -> Value {
    json!({
        "name": DEF,
        "description": "e2e wasm + agent flow",
        "steps": [
            {"name": "echo", "kind": {"type": "wasm", "command": "echo.wasm"}},
            {"name": "greet", "depends_on": ["echo"],
             "kind": {"type": "agent", "prompt": "报告 echo 步骤的输出"}},
        ],
    })
}

/// The node-local artifact root: `<node-data>/dag/<run_id>`.
fn run_dir(fleet: &Fleet, run: &str) -> std::path::PathBuf {
    fleet.node_data.join("dag").join(run)
}

/// Read a node artifact and parse it as JSON.
fn read_json(path: &std::path::Path) -> Value {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

/// Run the real control-plane CLI; returns (exit code, stdout+stderr).
fn cli(fleet: &Fleet, args: &[&str]) -> (i32, String) {
    let output = Command::new(sibling_bin(CLI_BIN))
        .args(["--server", &fleet.base, "--token", TOKEN])
        .args(args)
        .output()
        .expect("spawn opencoder-cli");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), text)
}

#[test]
fn dag_spec_dispatch_runs_wasm_and_agent_steps_to_done() {
    // The agent step is the only LLM consumer (the run session never
    // drains; the wasm step does not call the model).
    let stub = LlmStub::spawn_text(&[AGENT_TEXT, EXTRA_REPLY]);
    let tmp = tempfile::tempdir().unwrap();
    // Both fleet processes share this workdir config, so the pool root is
    // identical server-side (API writes) and node-side (freeze-on-accept).
    let fleet = Fleet::spawn_with_config(
        tmp.path(),
        stub.port(),
        json!({"dag": {"wasm_dir": tmp.path().join("wasm-pool")}}),
        "dag-flow-node",
    );
    publish(&fleet, "echo", &stdout_module_wat(ECHO_TEXT));

    // Save the definition through the real surface.
    let (status, definition) = fleet.http("POST", "/api/dag/defs", &json!({"spec": spec()}));
    assert_eq!(status, 200, "save def: {definition}");
    assert_eq!(definition["name"], DEF);
    assert_eq!(
        definition["spec"]["steps"].as_array().map(Vec::len),
        Some(2)
    );

    // Dispatch through the real CLI (exit-code + JSON stdout contract).
    let (code, out) = cli(
        &fleet,
        &[
            "dag",
            "dispatch",
            DEF,
            "--json",
            &format!(r#"{{"id":"{RUN}"}}"#),
        ],
    );
    assert_eq!(code, 0, "cli dispatch: {out}");
    let dispatched: Value = serde_json::from_str(&out).expect("cli dispatch json");
    assert_eq!(dispatched["run_id"], RUN);
    assert_eq!(dispatched["execution"]["kind"], "dag");
    assert_eq!(dispatched["execution"]["node_id"], fleet.node_id());

    let doc = fleet.wait_terminal(RUN);
    assert_eq!(doc["execution"]["status"], "done", "inspect: {doc}");
    assert_eq!(doc["result"]["run_id"], RUN);
    assert_eq!(doc["result"]["status"], "done");
    assert_eq!(doc["definition"]["name"], DEF);

    // Progress projection: both steps done, none left behind.
    let (status, progress) =
        fleet.http("GET", &format!("/api/dag/runs/{RUN}/progress"), &json!({}));
    assert_eq!(status, 200);
    assert_eq!(progress["execution_status"], "done");
    assert_eq!(progress["total"], 2);
    assert_eq!(progress["done"], 2);
    assert_eq!(progress["error"], 0);
    assert_eq!(progress["pending"], 0);

    // Wasm step: the module's stdout became the step output end to end.
    let (status, echo) = fleet.http(
        "GET",
        &format!("/api/dag/runs/{RUN}/steps/echo"),
        &json!({}),
    );
    assert_eq!(status, 200, "echo step: {echo}");
    assert_eq!(echo["status"], "done");
    assert!(
        echo["output"].is_null(),
        "this module only writes stdout, not output.json"
    );

    // Agent step: a real sub-session ran the prompt through the stub.
    let (status, greet) = fleet.http(
        "GET",
        &format!("/api/dag/runs/{RUN}/steps/greet"),
        &json!({}),
    );
    assert_eq!(status, 200, "greet step: {greet}");
    assert_eq!(greet["status"], "done");
    let session_id = greet["session_id"]
        .as_str()
        .expect("agent step publishes session_id")
        .to_string();

    // Node-local artifacts (the LOCKED step-io contract).
    let echo_out = std::fs::read_to_string(run_dir(&fleet, RUN).join("echo/output.txt"))
        .expect("echo output.txt");
    assert_eq!(echo_out.trim(), ECHO_TEXT);
    let echo_meta = read_json(&run_dir(&fleet, RUN).join("echo/meta.json"));
    assert_eq!(echo_meta["step"], "echo");
    assert_eq!(echo_meta["outcome"], "done");
    let greet_meta = read_json(&run_dir(&fleet, RUN).join("greet/meta.json"));
    assert_eq!(greet_meta["outcome"], "done");
    assert_eq!(greet_meta["session_id"], session_id.as_str());
    let live_pointer = read_json(&run_dir(&fleet, RUN).join("greet/session.json"));
    assert_eq!(live_pointer["session_id"], session_id.as_str());
    assert!(run_dir(&fleet, RUN).join("input.json").is_file());
    // Freeze-on-accept staged the referenced module into the library.
    assert!(fleet.node_data.join("dag/_modules/echo.wasm").is_file());

    // The step's sub-session is a real session: readable through the relay
    // (record is None → GET passes) with the step-scoped title.
    let (status, detail) = fleet.http("GET", &format!("/api/sessions/{session_id}"), &json!({}));
    assert_eq!(status, 200, "step session: {detail}");
    assert_eq!(detail["meta"]["title"], format!("dag/{RUN}/greet"));
    assert!(
        detail["messages"].to_string().contains(AGENT_TEXT),
        "step transcript must carry the scripted reply"
    );

    // Run-level SSE replay: lifecycle frames wrap the per-step events and
    // end with the terminal `run_finished`.
    let frames = sse_read(
        &fleet.base,
        &format!("/api/dag/runs/{RUN}/events"),
        TOKEN,
        0,
        None,
    );
    let kinds: Vec<&str> = frames.iter().map(|f| f.event.as_str()).collect();
    let started = kinds
        .iter()
        .position(|kind| *kind == "run_started")
        .unwrap();
    let first_step = kinds
        .iter()
        .position(|kind| *kind == "step_started")
        .unwrap();
    assert!(started < first_step, "frames: {kinds:?}");
    assert_eq!(kinds.last(), Some(&"stream_end"), "frames: {kinds:?}");
    let echo_done = frames
        .iter()
        .find(|f| f.event == "step_done" && f.data["step"] == "echo")
        .expect("echo step_done frame");
    assert_eq!(echo_done.data["payload"]["ok"], json!(true));
    let finished = frames
        .iter()
        .find(|frame| frame.event == "run_finished")
        .unwrap();
    assert_eq!(finished.data["payload"]["status"], "done");

    // Read the run back through the CLI as well (compat view contract).
    let (code, out) = cli(&fleet, &["dag", "runs", "get", RUN]);
    assert_eq!(code, 0, "cli runs get: {out}");
    let run_doc: Value = serde_json::from_str(&out).expect("cli run json");
    assert_eq!(run_doc["status"], "done");
    assert_eq!(run_doc["name"], DEF);

    // The only model call was the greet step (the wasm step never calls).
    let requests = stub.wait_for_requests(1);
    assert!(
        requests[0].contains("报告 echo 步骤的输出"),
        "prompt: {}",
        requests[0]
    );
}
