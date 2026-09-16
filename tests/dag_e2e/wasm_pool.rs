//! D2 — the versioned wasm-module pool over real HTTP and the
//! freeze-on-accept contract it feeds: publish v1 → run executes v1, a
//! new version → a *new* run executes v2, pointer rollback → a new run
//! executes v1 again, and an explicit `@v` pin executes its version even
//! though `current` points elsewhere. Also covers the REST error
//! contract (409 duplicate / 404 unknown / 400 bad base64) and the
//! binary download endpoint.

use crate::fixtures::{
    base64_encode, compile, download_bytes, pool_create_body, publish, stdout_module_wat,
};
use crate::support::fleet_proc::Fleet;
use crate::support::fleet_proc::TOKEN;
use crate::support::llm_stub::LlmStub;
use serde_json::json;

const NAME: &str = "echo";
const V1_TEXT: &str = "pool-e2e-v1";
const V2_TEXT: &str = "pool-e2e-v2";

/// Save a one-wasm-step def and dispatch `run_id`; returns the terminal doc.
fn run_echo(fleet: &Fleet, def: &str, command: &str, run: &str) -> serde_json::Value {
    let (status, body) = fleet.http(
        "POST",
        "/api/dag/defs",
        &json!({"spec": {"name": def, "steps": [
            {"name": "run", "kind": {"type": "wasm", "command": command}},
        ]}}),
    );
    assert_eq!(status, 200, "save def {def}: {body}");
    let (status, dispatched) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{def}/dispatch"),
        &json!({"id": run}),
    );
    assert_eq!(status, 202, "dispatch {run}: {dispatched}");
    fleet.wait_terminal(run)
}

/// The wasm step's captured stdout for `run`, trimmed.
fn step_output(fleet: &Fleet, run: &str) -> String {
    std::fs::read_to_string(fleet.node_data.join("dag").join(run).join("run/output.txt"))
        .unwrap_or_else(|error| panic!("output.txt for {run}: {error}"))
        .trim()
        .to_string()
}

#[test]
fn wasm_pool_versions_rollbacks_and_pins() {
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let pool_dir = tmp.path().join("wasm-pool");
    let fleet = Fleet::spawn_with_config(
        tmp.path(),
        stub.port(),
        json!({"dag": {"wasm_dir": pool_dir.clone()}}),
        "dag-pool-node",
    );

    // Empty pool + resolved NFS export root = the configured override dir.
    let (status, listed) = fleet.http("GET", "/api/dag/wasm", &json!({}));
    assert_eq!(status, 200);
    assert_eq!(listed["pools"].as_array().map(Vec::len), Some(0));
    let (status, nfs) = fleet.http("GET", "/api/dag/wasm/nfs", &json!({}));
    assert_eq!(status, 200);
    assert_eq!(nfs["root"].as_str(), Some(pool_dir.to_str().unwrap()));

    // v1 published; duplicate create is a 409; bad base64 a 400; an
    // unknown pool PUT a 404.
    let v1_bytes = compile(&stdout_module_wat(V1_TEXT));
    assert_eq!(publish(&fleet, NAME, &stdout_module_wat(V1_TEXT)), 1);
    let (status, body) = fleet.http(
        "POST",
        "/api/dag/wasm",
        &pool_create_body(NAME, "dup", &stdout_module_wat(V1_TEXT)),
    );
    assert_eq!(status, 409, "duplicate create: {body}");
    let (status, body) = fleet.http(
        "PUT",
        &format!("/api/dag/wasm/{NAME}"),
        &json!({"description": "v2", "wasm_b64": "not-base64!!"}),
    );
    assert_eq!(status, 400, "bad base64: {body}");
    let (status, body) = fleet.http(
        "PUT",
        "/api/dag/wasm/ghost",
        &json!({"description": "", "wasm_b64": base64_encode(&v1_bytes)}),
    );
    assert_eq!(status, 404, "unknown pool: {body}");

    // Def saved while current=v1: run 1 executes v1.
    assert_eq!(
        run_echo(&fleet, "e2e-pool-current", "echo.wasm", "dag-pool-run-1")["execution"]["status"],
        "done"
    );
    assert_eq!(step_output(&fleet, "dag-pool-run-1"), V1_TEXT);

    // v2 published: history keeps both versions, current points at v2,
    // and a NEW run picks v2 up.
    let (status, body) = fleet.http(
        "PUT",
        &format!("/api/dag/wasm/{NAME}"),
        &json!({"description": "v2", "wasm_b64": base64_encode(&compile(&stdout_module_wat(V2_TEXT)))}),
    );
    assert_eq!(status, 200, "put v2: {body}");
    assert_eq!(body["version"], 2);
    let (status, meta) = fleet.http("GET", &format!("/api/dag/wasm/{NAME}"), &json!({}));
    assert_eq!(status, 200);
    assert_eq!(meta["current"], 2);
    assert_eq!(meta["history"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        run_echo(&fleet, "e2e-pool-current", "echo.wasm", "dag-pool-run-2")["execution"]["status"],
        "done"
    );
    assert_eq!(
        step_output(&fleet, "dag-pool-run-2"),
        V2_TEXT,
        "new run executes current v2"
    );

    // Pointer-only rollback (v1 dir never left): a new run executes v1.
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/dag/wasm/{NAME}/rollback"),
        &json!({"version": 1}),
    );
    assert_eq!(status, 200, "rollback: {body}");
    assert_eq!(
        run_echo(&fleet, "e2e-pool-current", "echo.wasm", "dag-pool-run-3")["execution"]["status"],
        "done"
    );
    assert_eq!(
        step_output(&fleet, "dag-pool-run-3"),
        V1_TEXT,
        "rolled-back current executes v1"
    );

    // Explicit `@v2` pin executes v2 even though current is v1; the
    // frozen library keeps both pins side by side.
    assert_eq!(
        run_echo(&fleet, "e2e-pool-pinned", "echo@v2.wasm", "dag-pool-run-4")["execution"]
            ["status"],
        "done"
    );
    assert_eq!(
        step_output(&fleet, "dag-pool-run-4"),
        V2_TEXT,
        "explicit pin wins over current"
    );
    let modules = fleet.node_data.join("dag/_modules");
    assert_eq!(
        std::fs::read(modules.join("echo@v2.wasm")).expect("pinned v2 module"),
        compile(&stdout_module_wat(V2_TEXT))
    );
    assert_eq!(
        std::fs::read(modules.join("echo.wasm")).expect("unpinned v1 module"),
        v1_bytes
    );

    // Download endpoint serves the exact version bytes.
    let (status, bytes) = download_bytes(
        &fleet.base,
        &format!("/api/dag/wasm/{NAME}/versions/2/wasm.bin"),
        TOKEN,
    );
    assert_eq!(status, 200);
    assert_eq!(bytes, compile(&stdout_module_wat(V2_TEXT)));

    assert_eq!(
        stub.request_count(),
        0,
        "wasm-only runs never touch the LLM"
    );
}
