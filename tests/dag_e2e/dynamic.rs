//! Dynamic definition → fleet dispatch → node instances, HTTP pages and SSE.
use crate::fixtures::{args_echo_wat, publish};
use crate::support::{
    fleet_proc::{Fleet, TOKEN},
    http_util::http_text,
    llm_stub::{LlmStub, Script},
};
use serde_json::{json, Value};

fn get(fleet: &Fleet, path: &str) -> Value {
    let (status, value) = fleet.http("GET", path, &json!({}));
    assert_eq!(status, 200, "{path}: {value}");
    value
}
fn save_dispatch(fleet: &Fleet, name: &str, steps: Value, input: Value) -> String {
    let (status, value) = fleet.http(
        "POST",
        "/api/dag/defs",
        &json!({"spec":{"name":name,"steps":steps}}),
    );
    assert_eq!(status, 200, "save: {value}");
    let id = format!("dag-{name}");
    let (status, value) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{name}/dispatch"),
        &json!({"id":id,"input":input}),
    );
    assert_eq!(status, 202, "dispatch: {value}");
    id
}

#[test]
fn dynamic_wasm_instances_have_http_pages_isolated_argv_artifacts_and_replay() {
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(
        tmp.path(),
        stub.port(),
        json!({"dag":{"wasm_dir":tmp.path().join("wasm-pool")}}),
        "dynamic-http",
    );
    publish(&fleet, "argv", &args_echo_wat());
    let id = save_dispatch(
        &fleet,
        "dynamic-argv",
        json!([
            {"name":"process","kind":{"type":"dynamic","source":{"type":"input","pointer":"/items"},"template":{"type":"wasm","command":"argv.wasm --format json"}}}
        ]),
        json!({"items":[["--title","hello world"],["--target","web"]]}),
    );
    let terminal = fleet.wait_terminal(&id);
    assert_eq!(terminal["execution"]["status"], "done", "{terminal}");
    let base = format!("/api/dag/runs/{id}/steps/process/instances");
    let list = get(&fleet, &base);
    assert_eq!(list["total"], 2);
    assert_eq!(list["limit"], 100);
    assert_eq!(list["progress"]["done"], 2);
    let second = get(&fleet, &format!("{base}?offset=1&limit=500"));
    assert_eq!(second["limit"], 200);
    assert_eq!(second["instances"][0]["index"], 1);
    let first = get(&fleet, &format!("{base}/0"));
    assert_eq!(first["input"], json!(["--title", "hello world"]));
    let (status, log) = http_text(
        &fleet.base,
        "GET",
        &format!("{base}/0/events"),
        TOKEN,
        &[],
        None,
    );
    assert_eq!(status, 200);
    assert!(log.contains("hello world"), "{log}");
    assert!(!log.contains("--target"), "{log}");
    let (status, artifact) = http_text(
        &fleet.base,
        "GET",
        &format!("/api/executions/{id}/artifact?step=process&index=0&file=output.txt"),
        TOKEN,
        &[],
        None,
    );
    assert_eq!(status, 200, "{artifact}");
    assert_eq!(
        artifact,
        "argv.wasm\0--format\0json\0--title\0hello world\0"
    );
    let missing = fleet.http("GET", &format!("{base}/2"), &json!({}));
    assert_eq!(missing.0, 404);
    let progress = get(&fleet, &format!("/api/dag/runs/{id}/progress"));
    assert_eq!(progress["steps"].as_array().unwrap().len(), 1);
    assert_eq!(progress["steps"][0]["instances"]["done"], 2);
}

#[test]
fn runc_dynamic_agent_and_wasm_read_isolated_copies_and_argv() {
    if !std::process::Command::new("runc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("SKIP: runc unavailable");
        return;
    }
    let responder = Script::dynamic(|request| {
        let system = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "system")
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(system.contains("runc-common-how"));
        let item = if system.contains("runc-zero") {
            "zero"
        } else {
            assert!(system.contains("runc-one"));
            "one"
        };
        json!({"item":item}).to_string()
    });
    let stub = LlmStub::spawn(vec![responder.clone(), responder]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(
        tmp.path(),
        stub.port(),
        json!({"dag":{"agent_sandbox":"runc","wasm_dir":tmp.path().join("wasm-pool")}}),
        "dynamic-runc",
    );
    let prep = std::process::Command::new("bash")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/prepare-dag-rootfs.sh"
        ))
        .arg(fleet.node_data.join("dag/rootfs"))
        .output()
        .unwrap();
    assert!(
        prep.status.success(),
        "rootfs: {}",
        String::from_utf8_lossy(&prep.stderr)
    );
    publish(&fleet, "argv", &args_echo_wat());
    let id = save_dispatch(
        &fleet,
        "dynamic-runc",
        json!([
            {"name":"agents","kind":{"type":"dynamic","source":{"type":"input","pointer":"/text"},"template":{"type":"agent","prompt":"run","how_append":"runc-common-how"}}},
            {"name":"wasms","kind":{"type":"dynamic","source":{"type":"input","pointer":"/argv"},"template":{"type":"wasm","command":"argv.wasm --format json","sandbox":"runc"}}}
        ]),
        json!({"text":["runc-zero","runc-one"],"argv":[["hello world","--env","MODE=guest","--dir=/guest"],[]]}),
    );
    let terminal = fleet.wait_terminal(&id);
    assert_eq!(terminal["execution"]["status"], "done", "{terminal}");
    let base = format!("/api/dag/runs/{id}/steps/agents/instances");
    assert_eq!(
        get(&fleet, &format!("{base}/0"))["output"],
        json!({"item":"zero"})
    );
    assert_eq!(
        get(&fleet, &format!("{base}/1"))["output"],
        json!({"item":"one"})
    );
    let (status, log) = http_text(
        &fleet.base,
        "GET",
        &format!("{base}/0/events"),
        TOKEN,
        &[],
        None,
    );
    assert_eq!(status, 200);
    assert!(log.contains("zero"), "{log}");
    assert!(!log.contains("\\\"item\\\":\\\"one\\\""), "{log}");
    let root = fleet.node_data.join("dag").join(id);
    assert_eq!(
        std::fs::read_to_string(root.join("agents/instances/0/how.md")).unwrap(),
        "runc-common-how\n\nrunc-zero"
    );
    let output = std::fs::read_to_string(root.join("wasms/instances/0/output.txt")).unwrap();
    assert_eq!(
        &output.split('\0').collect::<Vec<_>>()[3..7],
        &["hello world", "--env", "MODE=guest", "--dir=/guest"]
    );
}
