//! The frozen `/layered` read surface against a real server + node: the view
//! keys the workbench codes against, per-layer detail, and the rule that a
//! layered route never serves another schema version.
use crate::fixtures::{cli_json, create, round, view, RUN};
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;
use serde_json::json;

#[test]
fn layered_view_and_rounds_read_a_real_projection() {
    // The canvas never sees a valid decision here: the surface under test is
    // the read route, not the dispatch.
    let stub = LlmStub::spawn(vec![]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet =
        Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "layered-surface-node");
    fleet.wait_ready(&["brain"]);
    create(&fleet, RUN);

    let view = view(&fleet, RUN);
    assert_eq!(view["schema_version"], json!(4));
    assert_eq!(view["run"]["run_id"], json!(RUN));
    assert_eq!(view["run"]["layer"], json!(0), "no layer is dispatched yet");
    assert_eq!(view["run"]["total_layers"], json!(2));
    assert_eq!(view["layers"], json!([["scan"], ["apply"]]));
    assert_eq!(view["plan"]["title"], json!("layered canvas"));
    assert_eq!(view["plan"]["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(view["operations"], json!([]));
    assert!(view["events"].is_array(), "{view}");
    // The scope is frozen at admission and deduplicated by capability id: both
    // nodes share one descriptor, so the canvas lists it once.
    let capabilities = view["capabilities"].as_array().unwrap();
    assert_eq!(capabilities.len(), 1, "{view}");
    let capability = &capabilities[0];
    assert_eq!(
        capability["capability_id"],
        json!("builtin-agent-act"),
        "{view}"
    );
    assert_eq!(capability["kind"], json!("agent"), "{view}");
    assert_eq!(capability["target"], json!("act"), "{view}");
    for field in ["version", "input_desc", "output_desc"] {
        assert!(
            capability[field]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "frozen scope {field}: {view}"
        );
    }

    // Layer detail is derived from the same plan: one row per layer node, with
    // the retry budget the plan declared.
    for (layer, node, attempts) in [(1, "scan", 2), (2, "apply", 3)] {
        let (status, body) = round(&fleet, RUN, layer);
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["schema_version"], json!(4));
        assert_eq!(body["layer"], json!(layer));
        assert!(body["phase"].is_string(), "{body}");
        assert_eq!(body["evidence_execution_ids"], json!([]));
        let nodes = body["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0]["node_id"], json!(node));
        assert_eq!(nodes[0]["status"], json!("pending"), "{body}");
        assert_eq!(nodes[0]["attempt"], json!(0));
        assert_eq!(nodes[0]["attempts"], json!(attempts));
        assert_eq!(nodes[0]["cancel_requested"], json!(false));
        assert!(nodes[0].get("inputs").is_none());
        assert!(nodes[0].get("summary").is_none());
        assert!(nodes[0]["execution_id"].is_null());
    }
    for layer in [0, 3] {
        let (status, body) = round(&fleet, RUN, layer);
        assert_eq!(status, 404, "{layer}: {body}");
        assert_eq!(body["error"], json!("layered layer not found"));
    }

    // An unknown canvas and a non-layered run are never served by the route.
    let (status, body) = fleet.http(
        "GET",
        "/api/brain/runs/brain-unknown-canvas/layered",
        &json!({}),
    );
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("layered run not found"));

    // Retired presentation routes are absent.
    for tail in ["view", "rounds/1"] {
        let (status, body) =
            fleet.http("GET", &format!("/api/brain/runs/{RUN}/{tail}"), &json!({}));
        assert_eq!(status, 404, "{body}");
    }

    // The layered command surface accepts exactly pause, resume and cancel.
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/brain/runs/{RUN}/commands"),
        &json!({"action":"interrupt"}),
    );
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("supported commands: pause, resume, cancel"),
        "{body}"
    );

    // The new CLI reads map to the same route, so workbench and CLI cannot
    // drift apart.
    let cli = cli_json(&fleet, &["brain", "runs", "layered", RUN]);
    assert_eq!(cli["schema_version"], json!(4));
    assert_eq!(cli["run"]["run_id"], json!(RUN));
    assert_eq!(cli["layers"], json!([["scan"], ["apply"]]));
    assert!(cli["run"]["total_layers"].is_u64(), "{cli}");
    let cli = cli_json(&fleet, &["brain", "runs", "layered-round", RUN, "2"]);
    assert_eq!(cli["layer"], json!(2));
    assert_eq!(cli["nodes"][0]["node_id"], json!("apply"));
    assert_eq!(cli["nodes"][0]["attempts"], json!(3));
}
