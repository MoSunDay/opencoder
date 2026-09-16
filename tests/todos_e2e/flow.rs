//! T1 — the full TODO face: save a template through `POST /api/todo/templates`,
//! run it, watch the real parent/child sessions drive dispatch → candidate →
//! accept → complete on the node, then verify the workflow record, the todo
//! items, the event stream and the node disk layout all agree.

use crate::fixtures::{
    assert_subsequence, install_template, read_json, stub_script, TEMPLATE, TODO_ID,
};
use crate::support::fleet_proc::{Fleet, TOKEN};
use serde_json::json;

pub const RUN: &str = "todos-e2e-flow-1";

#[test]
fn todo_template_runs_to_completed_with_passed_item() {
    // Exactly four scripted calls: parent dispatch, child candidate, parent
    // accept, parent complete (strays fall through to the extra reply).
    let stub = crate::support::llm_stub::LlmStub::spawn(stub_script());
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "todos-flow-node");
    fleet.wait_ready(&["todos"]);

    install_template(&fleet, TEMPLATE);

    // Run the template: 202 carrying the durable workflow id and node pin.
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/todo/templates/{TEMPLATE}/v1/run"),
        &json!({"id": RUN}),
    );
    assert_eq!(status, 202, "dispatch todos: {body}");
    assert_eq!(body["workflow_id"], json!(RUN));
    assert_eq!(body["execution"]["kind"], json!("todos"));
    assert_eq!(body["execution"]["node_id"], json!(fleet.node_id()));

    // The child prompt must carry the frozen todo instructions.
    let requests = stub.wait_for_requests(4);
    assert!(
        requests[1].contains("do it") && requests[1].contains("acceptance-criteria-mark"),
        "child prompt must carry the todo spec: {}",
        requests[1]
    );

    let doc = fleet.wait_terminal(RUN);
    assert_eq!(doc["execution"]["status"], json!("done"), "inspect: {doc}");
    assert_eq!(doc["result"]["workflow_id"], json!(RUN));
    assert_eq!(doc["result"]["status"], json!("completed"));
    assert_eq!(doc["result"]["todos"][TODO_ID]["status"], json!("passed"));
    assert_eq!(doc["workflow"]["workflow"]["id"], json!(RUN));
    assert_eq!(doc["workflow_initialization"], json!("ready"));

    // The node-side journal record lives in the node data dir.
    let record = read_json(
        &fleet
            .node_data
            .join("todos")
            .join(RUN)
            .join("execution.json"),
    );
    assert_eq!(record["assignment"]["index"]["kind"], json!("todos"));
    assert_eq!(record["assignment"]["index"]["status"], json!("done"));

    // Compat workflow record: completed workflow with a passed item.
    let (status, record) = fleet.http("GET", &format!("/api/todo/workflows/{RUN}"), &json!({}));
    assert_eq!(status, 200, "workflow record: {record}");
    assert_eq!(record["workflow"]["id"], json!(RUN));
    assert_eq!(record["workflow"]["status"], json!("completed"));
    assert_eq!(record["items"][0]["todo_id"], json!(TODO_ID));
    assert_eq!(record["items"][0]["status"], json!("passed"));

    // Item paging face agrees with the record.
    let (status, items) = fleet.http(
        "GET",
        &format!("/api/executions/{RUN}/todo-items"),
        &json!({}),
    );
    assert_eq!(status, 200, "todo items: {items}");
    assert_eq!(items["items"][0]["todo_id"], json!(TODO_ID));
    assert_eq!(items["items"][0]["status"], json!("passed"));

    // SSE replay: lifecycle events in order, then the terminal stream_end.
    let frames = crate::support::http_util::sse_read(
        &fleet.base,
        &format!("/api/todo/workflows/{RUN}/events"),
        TOKEN,
        0,
        None,
    );
    let kinds: Vec<&str> = frames.iter().map(|f| f.event.as_str()).collect();
    assert_subsequence(
        &kinds,
        &[
            "workflow_created",
            "todos_dispatched",
            "todo_candidate_ready",
            "todo_accepted",
            "workflow_completed",
        ],
        "todo event stream",
    );
    assert_eq!(kinds.last(), Some(&"stream_end"), "frames: {kinds:?}");
    assert_eq!(
        frames.last().map(|f| f.data["finished"].clone()),
        Some(json!(true))
    );
}
