//! Process-level e2e for the retained built-in review harness DAG and two
//! test-local review chains: startup seeding, host imports (`opencoder`
//! module: run_op/http_probe), fail-closed `dag.ops`, dispatch and five-field
//! execution index.

mod helpers;

use crate::fixtures::publish;
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::{LlmStub, Script};
use helpers::{
    dispatch_def, dispatch_wasm, op_log, op_step_wat, probe_server, save_def, script, step_json,
};
use serde_json::json;

const DEF_FULL: &str = "review-full-acceptance";
const SEED_HARNESS: &str = "review-harness-quick";
const DEF_CODE: &str = "review-code-quick";

fn code_spec() -> serde_json::Value {
    json!({"name":DEF_CODE,"max_concurrency":4,"steps":[
        {"name":"review-triage","kind":{"type":"agent","prompt":"定位评审范围"}},
        {"name":"review-risks","depends_on":["review-triage"],"kind":{"type":"agent","prompt":"识别风险点"}},
        {"name":"review-api-impact","depends_on":["review-triage"],"kind":{"type":"agent","prompt":"列出 API 影响面"}},
        {"name":"review-client","depends_on":["review-api-impact"],"kind":{"type":"agent","prompt":"检查客户端"}},
        {"name":"review-verdict","depends_on":["review-risks","review-client"],"kind":{"type":"agent","prompt":"给出评审结论"}},
        {"name":"review-report","depends_on":["review-verdict"],"kind":{"type":"agent","prompt":"输出 Markdown 评审报告"}}
    ]})
}

fn full_spec() -> serde_json::Value {
    json!({"name":DEF_FULL,"steps":[
        {"name":"env-eob-up","kind":{"type":"wasm","command":"env_eob_up.wasm"}},
        {"name":"env-baremetal-up","depends_on":["env-eob-up"],"kind":{"type":"wasm","command":"env_baremetal_up.wasm"}},
        {"name":"harness-full","depends_on":["env-baremetal-up"],"kind":{"type":"wasm","command":"harness_runner.wasm full"}}
    ]})
}

// ---------------------------------------------------------------------------
// R1 — the retained quick harness is seeded; test-local chains are not
// ---------------------------------------------------------------------------

#[test]
fn r1_review_defs_are_seeded_and_visible() {
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn(tmp.path(), stub.port(), "r1-seed-node");

    let (status, body) = fleet.http("GET", "/api/dag/defs", &json!({}));
    assert_eq!(status, 200, "{body}");
    let defs = body.as_array().expect("defs array");
    let by_name = |name: &str| {
        defs.iter()
            .find(|d| d["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing: {defs:?}"))
    };
    assert_eq!(
        defs.len(),
        1,
        "only the supported harness is seeded: {defs:?}"
    );
    assert_eq!(by_name(SEED_HARNESS)["id"], SEED_HARNESS);
    assert_eq!(
        by_name(SEED_HARNESS)["spec"]["steps"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// R2 — host imports: registered op (argv + env_keys + evidence), live
// probe, and the fail-closed unknown-op error
// ---------------------------------------------------------------------------

#[test]
fn r2_host_imports_run_ops_probe_and_fail_closed() {
    const SECRET_VAR: &str = "REVIEW_R2_SECRET";
    std::env::set_var(SECRET_VAR, "r2-secret-value");
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let cmd = script(
        tmp.path(),
        "r2-op.sh",
        "#!/bin/sh\necho \"argv=[$1] secret=$REVIEW_R2_SECRET\"\n",
    );
    let probe_port = probe_server();
    let fleet = Fleet::spawn_with_config(
        tmp.path(),
        stub.port(),
        json!({"dag": {"ops": {"noop": {
            "command": cmd, "env_keys": [SECRET_VAR], "timeout_secs": 60,
        }}}}),
        "r2-ops-node",
    );

    // Registered op + live probe → up, with op evidence on disk.
    let ok_json = r#"{"status":"up","evidence":"ops/noop.log"}"#;
    let wat = op_step_wat(
        "noop",
        "r2-args",
        Some(&format!("http://127.0.0.1:{probe_port}/ready")),
        "probe-op",
        ok_json,
        r#"{"status":"down"}"#,
    );
    publish(&fleet, "r2_probe_module", &wat);
    let doc = dispatch_wasm(
        &fleet,
        "e2e-r2-ops",
        "r2_probe_module.wasm",
        "dag-r2-run-1",
        "probe-op",
    );
    assert_eq!(doc["execution"]["status"], "done", "{doc:?}");
    assert_eq!(
        step_json(&fleet, "dag-r2-run-1", "probe-op")["status"],
        "up"
    );
    let log = op_log(&fleet, "dag-r2-run-1", "probe-op", "noop");
    assert!(log.contains("argv=[r2-args]"), "op log: {log}");
    assert!(
        log.contains("secret=r2-secret-value"),
        "env passthrough: {log}"
    );
    assert!(log.contains("# op noop exit=0"), "evidence header: {log}");

    // Unknown op id traps the guest → step Error → run failed (fail-closed).
    let ghost = op_step_wat(
        "ghost",
        "",
        None,
        "ghost-op",
        r#"{"status":"never"}"#,
        r#"{"status":"never"}"#,
    );
    publish(&fleet, "r2_ghost_module", &ghost);
    let doc = dispatch_wasm(
        &fleet,
        "e2e-r2-ghost",
        "r2_ghost_module.wasm",
        "dag-r2-run-2",
        "ghost-op",
    );
    assert_eq!(doc["execution"]["status"], "error", "{doc:?}");
    let step_error = doc["dag_steps"]["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s["error"].as_str())
        .collect::<Vec<_>>()
        .join("; ");
    assert!(
        step_error.contains("ghost"),
        "step error must name the unknown op: {doc:?}"
    );
    std::env::remove_var(SECRET_VAR);
}

// ---------------------------------------------------------------------------
// R3 — harness-quick: publish the harness module, dispatch the seeded
// def verbatim, assert the NDJSON evidence + structured verdict
// ---------------------------------------------------------------------------

#[test]
fn r3_harness_quick_chain_runs_the_seeded_def() {
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let cmd = script(
        tmp.path(),
        "harness.sh",
        concat!(
            "#!/bin/sh\n",
            "echo '{\"stage\": \"build\", \"status\": \"ok\"}'\n",
            "echo '{\"stage\": \"unit\", \"status\": \"ok\", \"detail\": \"7 passed\"}'\n",
        ),
    );
    let fleet = Fleet::spawn_with_config(
        tmp.path(),
        stub.port(),
        json!({"dag": {"ops": {"harness_run_quick": {"command": cmd, "timeout_secs": 120}}}}),
        "r3-harness-node",
    );

    let ok = r#"{"status":"passed","stages":2,"evidence":"ops/harness_run_quick.log"}"#;
    let wat = op_step_wat(
        "harness_run_quick",
        "quick",
        None,
        "harness-quick",
        ok,
        r#"{"status":"failed"}"#,
    );
    publish(&fleet, "harness_runner", &wat);

    let doc = dispatch_def(&fleet, SEED_HARNESS, "dag-r3-run-1");
    assert_eq!(doc["execution"]["status"], "done", "{doc:?}");
    let out = step_json(&fleet, "dag-r3-run-1", "harness-quick");
    assert_eq!(out["status"], "passed", "{out:?}");
    let log = op_log(&fleet, "dag-r3-run-1", "harness-quick", "harness_run_quick");
    assert!(
        log.contains("\"stage\": \"build\""),
        "ndjson evidence: {log}"
    );
    assert!(
        log.contains("\"detail\": \"7 passed\""),
        "ndjson evidence: {log}"
    );
}

// ---------------------------------------------------------------------------
// R4 — code-quick: six test-local agent steps, content-keyed stub,
// structured outputs and upstream-context passing
// ---------------------------------------------------------------------------

#[test]
fn r4_code_quick_agent_chain_passes_context_downstream() {
    let router = Script::dynamic(|body| {
        let prompt = body["messages"]
            .as_array()
            .and_then(|m| m.last())
            .and_then(|m| m["content"].as_str())
            .unwrap_or_default()
            .to_string();
        // Order matters: later prompts also quote earlier keywords.
        let reply = if prompt.contains("Markdown 评审报告") {
            "```json\n{\"report_md\": \"# 评审通过\"}\n```\n"
        } else if prompt.contains("给出评审结论") {
            "```json\n{\"verdict\": \"approve\", \"blocking\": []}\n```\n"
        } else if prompt.contains("客户端") {
            "```json\n{\"client_scope\": [\"web/e2e-items-panel\"], \"issues\": []}\n```\n"
        } else if prompt.contains("API 影响面") {
            "```json\n{\"api_impacts\": [{\"api\": \"GET /v1/items\", \"change\": \"resp-shape\", \"impact\": \"e2e-api-impact\"}]}\n```\n"
        } else if prompt.contains("风险点") {
            "```json\n{\"risks\": [{\"file\": \"src/e2e.rs\", \"why\": \"marker-risk\"}]}\n```\n"
        } else {
            "```json\n{\"scope\": [\"e2e-scope-marker\"], \"notes\": \"triage\"}\n```\n"
        };
        reply.to_string()
    });
    // The stub script is a FIFO: one entry per request. Six agent steps
    // (plus optional session-title calls) each need the router.
    let stub = LlmStub::spawn(vec![router; 10]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn(tmp.path(), stub.port(), "r4-code-node");

    let spec = code_spec();
    let steps = spec["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 6);
    assert_eq!(steps[1]["depends_on"], json!(["review-triage"]));
    assert_eq!(steps[2]["depends_on"], json!(["review-triage"]));
    assert_eq!(steps[3]["depends_on"], json!(["review-api-impact"]));
    assert_eq!(
        steps[4]["depends_on"],
        json!(["review-risks", "review-client"])
    );
    save_def(&fleet, spec);

    let doc = dispatch_def(&fleet, DEF_CODE, "dag-r4-run-1");
    assert_eq!(doc["execution"]["status"], "done", "{doc:?}");

    let steps = [
        ("review-triage", "scope"),
        ("review-risks", "risks"),
        ("review-api-impact", "api_impacts"),
        ("review-client", "client_scope"),
        ("review-client", "issues"),
        ("review-verdict", "verdict"),
        ("review-report", "report_md"),
    ];
    for (step, field) in steps {
        let out = step_json(&fleet, "dag-r4-run-1", step);
        assert!(
            !out[field].is_null(),
            "{step} output.json missing {field}: {out:?}"
        );
    }
    assert_eq!(
        step_json(&fleet, "dag-r4-run-1", "review-verdict")["verdict"],
        "approve"
    );

    // Context passing: the risks prompt must carry the triage step's
    // structured output (rendered as upstream step JSON), the client
    // prompt the api-impact output, the verdict prompt both branch heads
    // (client output plus triage via transitive ancestry), and the report
    // prompt the verdict.
    let requests = stub.wait_for_requests(6);
    let has = |needle: &str, marker: &str| {
        requests
            .iter()
            .any(|r| r.contains(needle) && r.contains(marker))
    };
    assert!(
        has("风险点", "e2e-scope-marker"),
        "upstream scope must reach the risks prompt"
    );
    assert!(
        has("客户端", "e2e-api-impact"),
        "api impact output must reach the client prompt"
    );
    assert!(
        has("客户端", "e2e-scope-marker"),
        "triage output must reach the client prompt via transitive ancestry"
    );
    assert!(
        has("给出评审结论", "web/e2e-items-panel"),
        "client scope must reach the verdict prompt"
    );
    assert!(
        has("给出评审结论", "e2e-scope-marker"),
        "triage output must reach the verdict prompt via transitive ancestry"
    );
    assert!(
        has("Markdown 评审报告", "approve")
            && requests
                .iter()
                .any(|r| r.contains("Markdown 评审报告") && r.contains("verdict")),
        "verdict must reach the report prompt"
    );

    // Run document projection: identity + lifecycle + placement.
    let (status, run_doc) = fleet.http("GET", "/api/dag/runs/dag-r4-run-1", &json!({}));
    assert_eq!(status, 200, "{run_doc:?}");
    for field in ["id", "name", "status", "created_at", "kind", "node_id"] {
        assert!(
            run_doc[field].is_string() || run_doc[field].is_number(),
            "{field}: {run_doc:?}"
        );
    }
    assert_eq!(run_doc["name"], DEF_CODE);

    // The protocol-locked five-field execution index also carries the run.
    let (_, page) = fleet.http("GET", "/api/executions?kind=dag&limit=50", &json!({}));
    let row = page["executions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == "dag-r4-run-1")
        .unwrap_or_else(|| panic!("run missing from the execution index: {page:?}"));
    for field in ["id", "created_at", "kind", "node_id", "status"] {
        assert!(!row[field].is_null(), "index field {field}: {row:?}");
    }
}

// ---------------------------------------------------------------------------
// R5 — full acceptance: publish all three modules, dispatch the test-local
// spec verbatim, every step done with artifacts + evidence in place
// ---------------------------------------------------------------------------

#[test]
fn r5_full_acceptance_dispatches_the_test_local_spec() {
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let probe_port = probe_server();
    let deploy = |name: &str| {
        script(
            tmp.path(),
            name,
            &format!(
                "#!/bin/sh\necho deploying\necho '{{\"endpoint\": \"http://127.0.0.1:{probe_port}\"}}'\n"
            ),
        )
    };
    let harness = script(
        tmp.path(),
        "harness_full.sh",
        concat!(
            "#!/bin/sh\n",
            "echo '{\"stage\": \"build\", \"status\": \"ok\"}'\n",
            "echo '{\"stage\": \"e2e\", \"status\": \"ok\", \"detail\": \"full run\"}'\n",
        ),
    );
    let fleet = Fleet::spawn_with_config(
        tmp.path(),
        stub.port(),
        json!({"dag": {"ops": {
            "env_eob_deploy": {"command": deploy("eob.sh"), "timeout_secs": 900},
            "env_baremetal_deploy": {"command": deploy("baremetal.sh"), "timeout_secs": 900},
            "harness_run_full": {"command": harness, "timeout_secs": 3600},
        }}}),
        "r5-full-node",
    );

    let probe_url = format!("http://127.0.0.1:{probe_port}");
    publish(
        &fleet,
        "env_eob_up",
        &op_step_wat(
            "env_eob_deploy",
            "",
            Some(&format!("{probe_url}/ready")),
            "env-eob-up",
            r#"{"status":"up","evidence":"ops/env_eob_deploy.log"}"#,
            r#"{"status":"down"}"#,
        ),
    );
    publish(
        &fleet,
        "env_baremetal_up",
        &op_step_wat(
            "env_baremetal_deploy",
            "",
            Some(&format!("{probe_url}/healthz")),
            "env-baremetal-up",
            r#"{"status":"up","evidence":"ops/env_baremetal_deploy.log"}"#,
            r#"{"status":"down"}"#,
        ),
    );
    publish(
        &fleet,
        "harness_runner",
        &op_step_wat(
            "harness_run_full",
            "full",
            None,
            "harness-full",
            r#"{"status":"passed","stages":2,"evidence":"ops/harness_run_full.log"}"#,
            r#"{"status":"failed"}"#,
        ),
    );

    let spec = full_spec();
    let steps = spec["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[1]["depends_on"], json!(["env-eob-up"]));
    assert_eq!(steps[2]["depends_on"], json!(["env-baremetal-up"]));
    save_def(&fleet, spec);
    let doc = dispatch_def(&fleet, DEF_FULL, "dag-r5-run-1");
    assert_eq!(doc["execution"]["status"], "done", "{doc:?}");

    assert_eq!(
        step_json(&fleet, "dag-r5-run-1", "env-eob-up")["status"],
        "up"
    );
    assert_eq!(
        step_json(&fleet, "dag-r5-run-1", "env-baremetal-up")["status"],
        "up"
    );
    assert_eq!(
        step_json(&fleet, "dag-r5-run-1", "harness-full")["status"],
        "passed"
    );

    for (step, op) in [
        ("env-eob-up", "env_eob_deploy"),
        ("env-baremetal-up", "env_baremetal_deploy"),
        ("harness-full", "harness_run_full"),
    ] {
        let log = op_log(&fleet, "dag-r5-run-1", step, op);
        assert!(
            log.contains(&format!("# op {op} exit=0")),
            "{op} evidence: {log}"
        );
    }
    let deploy_log = op_log(&fleet, "dag-r5-run-1", "env-eob-up", "env_eob_deploy");
    assert!(
        deploy_log.contains("\"endpoint\""),
        "tail json in evidence: {deploy_log}"
    );
}
