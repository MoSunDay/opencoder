//! Dispatch input `args` reach the wasm command line: the scheduler and
//! the compat dispatch surface both accept `input.args`, and the worker
//! appends it to every wasm step's command before execution — proven here
//! by a guest that echoes its own argv back as stdout.

use crate::fixtures::{args_echo_wat, publish};
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;
use serde_json::json;

const DEF: &str = "e2e-input-args";
const RUN: &str = "dag-e2e-input-args-1";
const ARGS: &str = "--date 2026-09-18 --mode strict";

/// Save the argv-echo def and dispatch it with `input.args`; returns the
/// terminal execution doc.
fn dispatch_with_args(fleet: &Fleet) -> serde_json::Value {
    let (status, body) = fleet.http(
        "POST",
        "/api/dag/defs",
        &json!({"spec": {"name": DEF, "steps": [
            {"name": "run", "kind": {"type": "wasm", "command": "argv.wasm"}},
        ]}}),
    );
    assert_eq!(status, 200, "save def: {body}");
    let (status, dispatched) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{DEF}/dispatch"),
        &json!({"id": RUN, "input": {"args": ARGS}}),
    );
    assert_eq!(status, 202, "dispatch: {dispatched}");
    fleet.wait_terminal(RUN)
}

#[test]
fn dispatch_input_args_append_to_the_wasm_command_line() {
    // Wasm-only DAG: no model call ever happens (same shape as wasm_pool).
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(
        tmp.path(),
        stub.port(),
        json!({"dag": {"wasm_dir": tmp.path().join("wasm-pool")}}),
        "dag-input-args-node",
    );
    publish(&fleet, "argv", &args_echo_wat());

    let doc = dispatch_with_args(&fleet);
    assert_eq!(doc["execution"]["status"], "done", "inspect: {doc}");

    // The guest dumped its whole argv buffer: argv[0] is the module token,
    // then the appended `input.args` as separate command-line tokens.
    let raw = std::fs::read(fleet.node_data.join("dag").join(RUN).join("run/output.txt"))
        .unwrap_or_else(|error| panic!("output.txt for {RUN}: {error}"));
    let argv: Vec<&str> = std::str::from_utf8(&raw)
        .expect("argv dump is utf-8")
        .split('\0')
        .filter(|token| !token.is_empty())
        .collect();
    assert_eq!(argv.first(), Some(&"argv.wasm"), "argv: {argv:?}");
    assert_eq!(
        &argv[1..],
        &["--date", "2026-09-18", "--mode", "strict"],
        "appended args must land after the module token: {argv:?}"
    );
}
