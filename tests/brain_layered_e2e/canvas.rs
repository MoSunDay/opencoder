//! One live canvas turn end to end: the node-local activation decides layer 1,
//! the layer barrier holds layer 2 until the child terminal folds, the closing
//! activation completes the run, and every node body stays with its child.
//!
//! The activation runs in the read-only OCI container the node builds once per
//! generation, so the scenario skips where `runc` is not provisioned.

use crate::fixtures::{
    cli_json, create, operation, round, runc_available, script, wait_phase, wait_view, CHILD_TEXT,
    RUN, SUMMARY,
};
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;
use serde_json::{json, Value};

/// One canvas event as the ordering assertion reads it: type, layer, node.
type CanvasEvent = (String, u64, String);

/// The durable canvas order: only the structural commits of the state machine.
/// `decision_started` is activation bookkeeping (one per claimed generation)
/// and `operation_admitted` is delivery-ordered (the node reports the child
/// receipt asynchronously), so neither is a canvas fact.
fn canvas_events(view: &Value) -> Vec<CanvasEvent> {
    view["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|event| {
            let event_type = event["event_type"].as_str()?;
            if matches!(event_type, "operation_admitted" | "decision_started") {
                return None;
            }
            Some((
                event_type.to_owned(),
                event["layer"].as_u64().unwrap_or_default(),
                event["node_id"].as_str().unwrap_or_default().to_owned(),
            ))
        })
        .collect()
}

/// The node row of one layer detail.
fn row<'a>(detail: &'a Value, node_id: &str) -> &'a Value {
    detail["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["node_id"] == json!(node_id)))
        .unwrap_or_else(|| panic!("no node row {node_id} in {detail}"))
}

#[test]
fn layered_canvas_holds_the_barrier_then_completes_through_the_closing_activation() {
    if !runc_available() {
        eprintln!("SKIP: runc unavailable");
        return;
    }
    // One request-aware responder serves the three activations (two layer
    // decisions, one closing) and the two leaf turns.
    let stub = LlmStub::spawn(script());
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "layered-canvas-node");
    fleet.wait_ready(&["brain"]);
    create(&fleet, RUN);

    // Layer 1 is dispatched alone: `apply` may not be scheduled while `scan` is
    // still in flight, which is the whole point of a layer barrier.
    let first = wait_view(&fleet, RUN, "layer 1 dispatched", 300, |view| {
        view["run"]["layer"] == json!(1) && view["run"]["phase"] == json!("waiting")
    });
    assert!(
        first["run"]["generation"].as_u64().unwrap_or_default() >= 1,
        "a decided canvas has claimed a generation: {first}"
    );
    assert_eq!(first["run"]["total_layers"], json!(2), "{first}");
    assert_eq!(first["layers"], json!([["scan"], ["apply"]]), "{first}");
    assert_eq!(first["operations"].as_array().unwrap().len(), 1, "{first}");
    let scan = operation(&first, "scan").expect("scan dispatched").clone();
    assert_eq!(scan["operation_id"], json!(format!("{RUN}#l1#scan#a1")));
    assert_eq!(scan["run_id"], json!(RUN));
    assert_eq!(scan["layer"], json!(1));
    assert_eq!(scan["node_id"], json!("scan"));
    assert_eq!(scan["capability_id"], json!("builtin-agent-act"));
    assert_eq!(scan["execution_kind"], json!("agent"));
    assert_eq!(scan["attempt"], json!(1));
    assert_eq!(scan["cancel_requested"], json!(false));
    assert!(
        scan["execution_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "{scan}"
    );
    assert!(
        operation(&first, "apply").is_none(),
        "layer 2 must wait for the barrier: {first}"
    );

    // The barrier folds `scan`, the wake decides layer 2, and the closing
    // activation completes the run with the bounded summary.
    let second = wait_view(&fleet, RUN, "layer 2 dispatched", 300, |view| {
        view["run"]["layer"] == json!(2) && operation(view, "apply").is_some()
    });
    assert!(
        second["run"]["generation"] != first["run"]["generation"],
        "the wake claims a new generation per layer: {second}"
    );
    let apply = operation(&second, "apply")
        .expect("apply dispatched")
        .clone();
    assert_eq!(apply["operation_id"], json!(format!("{RUN}#l2#apply#a1")));
    assert_eq!(apply["layer"], json!(2));
    assert_eq!(apply["execution_kind"], json!("agent"));

    let done = wait_phase(
        &fleet,
        RUN,
        "closing decision completes the canvas",
        600,
        &["completed"],
    );
    assert_eq!(done["run"]["phase"], json!("completed"), "{done}");
    assert_eq!(done["run"]["layer"], json!(2), "{done}");
    assert_eq!(done["run"]["total_layers"], json!(2), "{done}");
    assert_eq!(done["run"]["summary"], json!(SUMMARY), "{done}");
    assert_eq!(done["run"]["error"], json!(null), "{done}");

    // Durable order: one dispatch per layer, a barrier between the layers, and
    // the closing decision last.
    assert_eq!(
        canvas_events(&done),
        vec![
            ("run_created".into(), 0, String::new()),
            ("layer_started".into(), 1, String::new()),
            ("node_dispatched".into(), 1, "scan".into()),
            ("operation_terminal".into(), 1, "scan".into()),
            ("layer_barrier_reached".into(), 1, String::new()),
            ("layer_started".into(), 2, String::new()),
            ("node_dispatched".into(), 2, "apply".into()),
            ("operation_terminal".into(), 2, "apply".into()),
            ("layer_barrier_reached".into(), 2, String::new()),
            ("run_completed".into(), 2, String::new()),
        ],
        "canvas event order: {done}"
    );

    // Every node keeps its body with its own child execution: the root only
    // references the child, and the child owns the capability result.
    let operations = done["operations"].as_array().expect("operations").clone();
    assert_eq!(operations.len(), 2, "{done}");
    for (node_id, layer) in [("scan", 1_u64), ("apply", 2)] {
        let op = operation(&done, node_id)
            .unwrap_or_else(|| panic!("{node_id} operation: {done}"))
            .clone();
        assert_eq!(
            op["operation_id"],
            json!(format!("{RUN}#l{layer}#{node_id}#a1"))
        );
        assert_eq!(op["attempt"], json!(1), "{op}");
        assert_eq!(op["status"], json!("done"), "{op}");
        assert_eq!(op["cancel_requested"], json!(false), "{op}");
        let child = op["execution_id"]
            .as_str()
            .expect("child execution")
            .to_owned();
        let doc = fleet.wait_terminal(&child);
        assert_eq!(
            doc["execution"]["status"],
            json!("done"),
            "child {child}: {doc}"
        );
        assert_eq!(
            doc["execution"]["kind"],
            json!("agent"),
            "child {child}: {doc}"
        );
        let input = &doc["request"]["input"];
        assert_eq!(input["schema_version"], json!(4), "child {child}: {doc}");
        assert_eq!(input["brain_layered"]["run_id"], json!(RUN), "{doc}");
        assert_eq!(input["brain_layered"]["node_id"], json!(node_id), "{doc}");
        assert_eq!(input["brain_layered"]["layer"], json!(layer), "{doc}");
        assert_eq!(
            input["brain_layered"]["operation_id"], op["operation_id"],
            "{doc}"
        );
        assert_eq!(input["layered_inputs"], json!({}), "{doc}");
        assert!(
            input["prompt"]
                .as_str()
                .unwrap_or_default()
                .contains("bounded capability task"),
            "{doc}"
        );
        assert_eq!(
            doc["result"]["scheduler_output"],
            json!(CHILD_TEXT),
            "child {child}: {doc}"
        );
    }
    // The root projection carries references and bounded reasons, never a node
    // body: the child result text appears nowhere in it.
    assert!(
        !done.to_string().contains(CHILD_TEXT),
        "root view leaked a child body: {done}"
    );

    // The per-layer detail of the finished canvas. Layer 1 stays past its
    // barrier (the terminal fold is its last decision) and layer 2 carries the
    // closing decision, with one frozen node row each.
    let (status, body) = round(&fleet, RUN, 1);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["layer"], json!(1), "{body}");
    assert_eq!(
        body["phase"],
        json!("deciding"),
        "layer 1 folded past its barrier: {body}"
    );
    assert_eq!(body["evidence_execution_ids"], json!([]), "{body}");
    let scan_row = row(&body, "scan");
    assert_eq!(scan_row["status"], json!("done"), "{body}");
    assert_eq!(scan_row["attempt"], json!(1), "{body}");
    assert_eq!(scan_row["attempts"], json!(2), "{body}");
    assert_eq!(scan_row["execution_id"], scan["execution_id"], "{body}");
    assert_eq!(scan_row["inputs"], json!({}), "{body}");
    assert_eq!(scan_row["cancel_requested"], json!(false), "{body}");
    assert_eq!(scan_row["summary"], json!(CHILD_TEXT), "{body}");
    assert!(body["nodes"].as_array().unwrap().len() == 1, "{body}");

    let (status, body) = round(&fleet, RUN, 2);
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["phase"],
        json!("completed"),
        "the closing decision is the last layer-2 event: {body}"
    );
    assert_eq!(
        body["reason"],
        json!("e2e closing decision after every layer"),
        "{body}"
    );
    let apply_row = row(&body, "apply");
    assert_eq!(apply_row["status"], json!("done"), "{body}");
    assert_eq!(apply_row["attempt"], json!(1), "{body}");
    assert_eq!(apply_row["attempts"], json!(3), "{body}");
    assert_eq!(apply_row["execution_id"], apply["execution_id"], "{body}");
    assert_eq!(apply_row["summary"], json!(CHILD_TEXT), "{body}");

    // The CLI face of the finished canvas is the same projection.
    let cli = cli_json(&fleet, &["brain", "runs", "layered", RUN]);
    assert_eq!(cli["run"]["phase"], json!("completed"), "{cli}");
    assert_eq!(cli["run"]["summary"], json!(SUMMARY), "{cli}");

    // Every model call is accounted for: the activations carry the v4 contract
    // prompt and the leaf turns carry the bounded node prompt, so the canvas
    // stays a bounded number of calls.
    let requests = stub.wait_for_requests(5);
    let activations = requests
        .iter()
        .filter(|body| body.contains("You are an event-driven brain scheduler, schema_version 4."))
        .count();
    assert!(
        activations >= 3,
        "expected three layered activations, got {activations} of {} request(s)",
        requests.len()
    );
    let leaves = requests
        .iter()
        .filter(|body| body.contains("bounded capability task"))
        .count();
    assert!(
        leaves >= 2,
        "expected two leaf turns, got {leaves} of {} request(s)",
        requests.len()
    );
    let total = stub.request_count();
    assert!(
        total <= 8,
        "the canvas must stay bounded, got {total} model calls"
    );
}
