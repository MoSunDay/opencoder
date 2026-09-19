//! T2 + T3 — lifecycle control: a durable interrupt survives a node crash
//! and resumes to completion (T2), and a child LLM outage folds the todo to
//! failed with a runtime suspension instead of wedging the workflow (T3).

use crate::fixtures::{
    assert_subsequence, install_template, read_json, CHILD_CANDIDATE, PARENT_ACCEPT,
    PARENT_COMPLETE, PARENT_DISPATCH, TEMPLATE, TODO_ID,
};
use crate::support::fleet_proc::Fleet;
use crate::support::http_util::sse_read;
use crate::support::llm_stub::{LlmStub, Script};
use serde_json::json;

const INTERRUPT_RUN: &str = "todos-e2e-lifecycle-1";
const FAIL_RUN: &str = "todos-e2e-fail-1";

/// T2 — interrupt while the parent decision is parked, crash + respawn the
/// node, resume, and verify the workflow completes with the transcript
/// (interrupt → resumed) intact.
#[test]
fn interrupt_survives_node_restart_and_resumes_to_done() {
    // Hold parks the parent's first decision inside the stub, giving the
    // interrupt a stable "drain in flight" point.
    let stub = LlmStub::spawn(vec![
        Script::Hold,
        Script::Text(PARENT_DISPATCH.into()),
        Script::Text(CHILD_CANDIDATE.into()),
        Script::Text(PARENT_ACCEPT.into()),
        Script::Text(PARENT_COMPLETE.into()),
    ]);
    let tmp = tempfile::tempdir().unwrap();
    let mut fleet =
        Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "todos-lifecycle-node");
    fleet.wait_ready(&["todos"]);

    install_template(&fleet, TEMPLATE);
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/todo/templates/{TEMPLATE}/v1/run"),
        &json!({"id": INTERRUPT_RUN}),
    );
    assert_eq!(status, 202, "dispatch: {body}");
    assert_eq!(body["workflow_id"], json!(INTERRUPT_RUN));

    stub.wait_until_entered();
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/todo/workflows/{INTERRUPT_RUN}/interrupt"),
        &json!({}),
    );
    // The command acknowledges with the in-flight drain state; the durable
    // interrupt lands asynchronously.
    assert_eq!(status, 200, "interrupt: {body}");
    assert_eq!(body["status"], json!("cancelling"));

    let doc = fleet.wait_status(INTERRUPT_RUN, "interrupted status", 180, |body| {
        body["execution"]["status"] == json!("interrupted")
    });
    assert_eq!(doc["workflow"]["workflow"]["status"], json!("suspended"));
    // Draining the parked request folds the stub thread cleanly; the
    // interrupted session no longer consumes the reply.
    stub.release();

    // Node crash + respawn against the same data dir: the node id and the
    // journal (including the suspended workflow) survive.
    let node_id = fleet.node_id();
    fleet.respawn_agent();
    assert_eq!(fleet.node_id(), node_id, "node id survives the restart");

    // Resume re-enqueues the suspended workflow; the remaining script
    // entries drive it to completion. If restart recovery already resumed
    // the workflow the command may legitimately race into a 409.
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/todo/workflows/{INTERRUPT_RUN}/resume"),
        &json!({}),
    );
    assert!(
        status == 200 || status == 409,
        "resume reply: {status} {body}"
    );

    let doc = fleet.wait_terminal(INTERRUPT_RUN);
    assert_eq!(doc["execution"]["status"], json!("done"), "inspect: {doc}");
    assert_eq!(doc["result"]["status"], json!("completed"));
    assert_eq!(doc["result"]["todos"][TODO_ID]["status"], json!("passed"));

    // The SSE transcript survived the crash: the in-flight drain race folds
    // to either the clean interrupt commit or a runtime_error suspension,
    // then resume re-drives the workflow to completion.
    let frames = sse_read(
        &fleet.base,
        &format!("/api/todo/workflows/{INTERRUPT_RUN}/events"),
        crate::support::fleet_proc::TOKEN,
        0,
        None,
    );
    let kinds: Vec<&str> = frames.iter().map(|f| f.event.as_str()).collect();
    let suspended_at = kinds
        .iter()
        .position(|k| *k == "workflow_interrupted" || *k == "runtime_error")
        .expect("interrupt must suspend the workflow");
    assert_subsequence(
        &kinds[suspended_at..],
        &["workflow_resumed", "todos_dispatched", "workflow_completed"],
        "resume event stream",
    );
    assert_eq!(kinds.last(), Some(&"stream_end"), "frames: {kinds:?}");
}

/// T3 — the child session's model call fails with a non-200: the todo folds
/// to failed (max_attempts=1), the next parent decision falls through to the
/// stub's extra reply, and the runtime suspends the workflow with an
/// execution error instead of looping.
#[test]
fn child_model_failure_suspends_workflow_with_failed_todo() {
    let stub = LlmStub::spawn(vec![
        Script::Text(PARENT_DISPATCH.into()),
        Script::Fail(400, "todo child model outage".into()),
    ]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "todos-fail-node");
    fleet.wait_ready(&["todos"]);

    install_template(&fleet, TEMPLATE);
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/todo/templates/{TEMPLATE}/v1/run"),
        &json!({"id": FAIL_RUN}),
    );
    assert_eq!(status, 202, "dispatch: {body}");

    // dispatch + failing child + the parent re-ask cycle (1 ask + 2 parse
    // retries, all fed the stub's extra reply) — the folded todo must not
    // wedge the runtime.
    let requests = stub.wait_for_requests(5);
    assert!(
        requests[1].contains("do it") && requests[1].contains("acceptance-criteria-mark"),
        "second call must be the child session: {}",
        requests[1]
    );
    // The parent re-ask cycle: first re-ask after the fold, then the
    // parse-correction retries against the stub's plain-text extra reply.
    assert!(
        requests[2..]
            .iter()
            .all(|body| body.contains("Decide the next workflow operation")),
        "requests after the fold must be parent re-asks: {:?}",
        &requests[2..]
    );
    assert!(
        requests[requests.len() - 1].contains("could not be parsed as the required JSON object"),
        "parent must re-ask on the unparsable extra reply: {}",
        requests[requests.len() - 1]
    );

    let doc = fleet.wait_terminal(FAIL_RUN);
    assert_eq!(doc["execution"]["status"], json!("error"), "inspect: {doc}");
    // The workflow record folded to suspended with the failing todo.
    assert_eq!(doc["workflow"]["workflow"]["status"], json!("suspended"));
    assert_eq!(doc["workflow"]["items"][0]["status"], json!("failed"));

    // The store-backed todo item folded to failed with the child outage.
    let (status, items) = fleet.http(
        "GET",
        &format!("/api/executions/{FAIL_RUN}/todo-items"),
        &json!({}),
    );
    assert_eq!(status, 200, "todo items: {items}");
    assert_eq!(items["items"][0]["todo_id"], json!(TODO_ID));
    assert_eq!(items["items"][0]["status"], json!("failed"));
    assert!(
        items["items"][0]["last_error"]
            .as_str()
            .unwrap_or_default()
            .contains("todo child model outage"),
        "failure reason persisted: {}",
        items["items"][0]
    );

    let record = read_json(
        &fleet
            .node_data
            .join("todos")
            .join(FAIL_RUN)
            .join("execution.json"),
    );
    assert_eq!(record["assignment"]["index"]["status"], json!("error"));

    // Event fold: dispatched → todo failed → runtime suspension.
    let frames = sse_read(
        &fleet.base,
        &format!("/api/todo/workflows/{FAIL_RUN}/events"),
        crate::support::fleet_proc::TOKEN,
        0,
        None,
    );
    let kinds: Vec<&str> = frames.iter().map(|f| f.event.as_str()).collect();
    assert_subsequence(
        &kinds,
        &["todos_dispatched", "todo_execution_failed", "runtime_error"],
        "failure event stream",
    );
    assert_eq!(kinds.last(), Some(&"stream_end"), "frames: {kinds:?}");
}
