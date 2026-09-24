//! V4 layered roots: one wake per generation, one decision per layer context,
//! attempt identities that retry without re-deciding, and a closing context that
//! finalizes the root.
#[path = "scheduler_v4/client.rs"]
mod client;
#[path = "scheduler_v4/control.rs"]
mod control;
#[path = "scheduler_v4/plan.rs"]
mod plan;
mod support;

use client::LayeredClient;
use opencoder_core::{brain::layered::*, fleet::*};
use opencoder_node::fleet::NodeService;
use opencoder_worker::Worker;
use serde_json::{json, Value};
use std::sync::Arc;
use support::*;

fn root(id: &str) -> ExecutionRef {
    ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Brain,
    }
}

/// Create one v4 root the way control does: the canvas is frozen in the
/// execution input, so the node never resolves a plan of its own.
async fn create(node: &Worker, id: &str) {
    let input = json!({"schema_version": 7, "layered_request": plan::request(id)});
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(node, id, ExecutionKind::Brain, input, Some(json!({}))),
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    settled(node, id).await;
}

#[tokio::test]
async fn root_emits_one_layered_wake_until_control_acknowledges_it() {
    let (_config, _home) = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), mock()).await;
    let id = "brain-v4-wake";
    create(&node, id).await;
    let wakes = control::actions(&control::frames(&node).await, "layered_wake");
    assert_eq!(wakes.len(), 1, "one wake per generation: {wakes:?}");
    let generation = wakes[0]["generation"].as_u64().expect("wake generation");
    control::ok(
        &node,
        id,
        "layered_wake_ack",
        json!({"generation": generation}),
    )
    .await;
    assert!(control::actions(&control::frames(&node).await, "layered_wake").is_empty());
    // Pause and resume own the generation: a resumed root earns a new wake, and
    // the old acknowledgement must not silence it.
    for action in ["pause", "resume"] {
        let reply = node
            .handle(NodeOperation::Brain {
                execution: root(id),
                action: action.into(),
                input: Value::Null,
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
    }
    let resumed = control::wait_wake(&node).await;
    let resumed_generation = resumed["generation"].as_u64().expect("resumed generation");
    assert!(resumed_generation > generation, "{resumed:?}");
    // Replaying an acknowledgement never moves the cursor backwards, so the
    // wake stays closed once the newest generation is acknowledged.
    for value in [resumed_generation, generation] {
        control::ok(&node, id, "layered_wake_ack", json!({"generation": value})).await;
    }
    assert!(control::actions(&control::frames(&node).await, "layered_wake").is_empty());
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_decision_is_corrected_before_any_capability_is_dispatched() {
    let (_config, _home) = isolated_config();
    let client = Arc::new(LayeredClient::with([json!({
        "decision":"dispatch_layer","layer":2,"assignments":[],"reason":"skip the first layer"
    })]));
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), client.clone()).await;
    let id = "brain-v6-correction";
    create(&node, id).await;
    control::decide_next_layer(&node, id).await;
    let waiting = control::wait_phase(&node, id, LayeredPhase::Waiting).await;
    assert_eq!(waiting.run.layer, 1);
    assert_eq!(waiting.run.activation, 1);
    assert_eq!(client.decisions(), 2);
    assert!(client.instructions()[1].contains("Previous decision rejected"));
    assert_eq!(control::wait_dispatch(&node, 1).await.len(), 1);
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn three_invalid_decisions_block_without_dispatching_a_capability() {
    let (_config, _home) = isolated_config();
    let invalid = json!({"decision":"dispatch_layer","layer":2,"assignments":[],"reason":"skip"});
    let client = Arc::new(LayeredClient::with([
        invalid.clone(),
        invalid.clone(),
        invalid,
    ]));
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), client.clone()).await;
    let id = "brain-v6-exhausted";
    create(&node, id).await;
    control::decide_next_layer(&node, id).await;
    let blocked = control::wait_phase(&node, id, LayeredPhase::Blocked).await;
    assert_eq!(client.decisions(), 3);
    assert_eq!(blocked.run.activation, 0);
    assert!(blocked
        .run
        .error
        .as_deref()
        .unwrap_or("")
        .contains("after 3 attempts"));
    assert!(control::actions(&control::frames(&node).await, "layered_dispatch").is_empty());
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn layered_context_requeues_the_idle_root_for_its_layer_decision() {
    let (_config, _home) = isolated_config();
    let client = Arc::new(LayeredClient::new());
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), client.clone()).await;
    let id = "brain-v4-context";
    create(&node, id).await;
    // No layer can be decided before its context is installed: the canvas of a
    // root is control data, never a node-local plan lookup.
    assert_eq!(client.decisions(), 0, "{:?}", client.instructions());
    assert!(control::actions(&control::frames(&node).await, "layered_dispatch").is_empty());
    let context = control::decide_next_layer(&node, id).await;
    assert_eq!(context.layer, 0);
    assert_eq!(context.request.plan.nodes.len(), 2, "{context:?}");
    // Installing a context only changes the durable projection, so the node
    // must requeue its own activation before one layer can be decided.
    let waiting = control::wait_phase(&node, id, LayeredPhase::Waiting).await;
    assert_eq!(waiting.run.layer, 1, "{waiting:?}");
    assert_eq!(client.decisions(), 1, "{:?}", client.instructions());
    let dispatches = control::wait_dispatch(&node, 1).await;
    assert_eq!(dispatches.len(), 1, "{dispatches:?}");
    let operation = plan::operation(&dispatches[0]);
    assert_eq!(
        operation.operation_id,
        format!("{id}#l1#scan#visit1#cap-scan#a1")
    );
    assert_eq!(operation.attempt, 1);
    assert_eq!(operation.execution_kind, ExecutionKind::Agent);
    assert!(
        operation.execution_id.starts_with("agent-"),
        "{operation:?}"
    );
    assert_eq!(operation.status, LayeredOperationStatus::Creating);
    assert_eq!(
        dispatches[0]["assignment"]["inputs"]["repo"],
        json!({"kind": "root", "name": "repo"})
    );
    // The durable intent authorizes exactly its own triple; a mutated operation
    // is fenced instead of creating a child.
    control::authorize(&node, id, &dispatches[0]).await;
    let mut forged = dispatches[0].clone();
    forged["operation"]["cancel_requested"] = json!(true);
    let denied = control::rpc(&node, root(id), "layered_authorize", forged).await;
    assert_eq!(denied.status, 409, "{denied:?}");
    // The dispatch replays until control acknowledges that one operation.
    control::ack_dispatch(&node, id, &operation.operation_id).await;
    assert!(control::actions(&control::frames(&node).await, "layered_dispatch").is_empty());
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn failure_wakes_the_brain_and_reflection_creates_a_distinct_durable_visit() {
    let (_config, _home) = isolated_config();
    let client = Arc::new(LayeredClient::new());
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), client.clone()).await;
    let id = "brain-reflection";
    create(&node, id).await;
    control::decide_next_layer(&node, id).await;
    let first = control::wait_dispatch(&node, 1).await.remove(0);
    let original = plan::operation(&first);
    control::authorize(&node, id, &first).await;
    control::admit(&node, id, &original.operation_id).await;
    control::ack_dispatch(&node, id, &original.operation_id).await;
    let old_notice = control::finish(&node, id, &first, LayeredOperationStatus::Error).await;
    let ready = control::wait_phase(&node, id, LayeredPhase::Ready).await;
    assert_eq!(ready.operations.len(), 1, "no implicit retry");
    assert_eq!(client.decisions(), 1);
    control::decide_next_layer(&node, id).await;
    let returned = control::wait_dispatch(&node, 1).await.remove(0);
    let next = plan::operation(&returned);
    assert_eq!(next.round, 2);
    assert_eq!(next.activation, 2);
    assert_ne!(next.execution_id, original.execution_id);
    control::authorize(&node, id, &returned).await;
    let duplicate = control::ok(&node, id, "layered_terminal", json!(old_notice)).await;
    assert_eq!(duplicate["duplicate"], true);
    control::admit(&node, id, &next.operation_id).await;
    control::ack_dispatch(&node, id, &next.operation_id).await;
    control::finish(&node, id, &returned, LayeredOperationStatus::Done).await;
    control::complete_layer(&node, id).await;
    control::decide_next_layer(&node, id).await;
    let completed = control::wait_phase(&node, id, LayeredPhase::Completed).await;
    assert_eq!(completed.run.round, 2);
    assert_eq!(completed.operations.len(), 3);
    let events = control::events(&node, id).await;
    let reflection = events
        .iter()
        .find(|e| e["decision_summary"] == "reflect_and_return")
        .unwrap();
    assert_eq!(reflection["round"], 2);
    assert!(reflection["reflection"]
        .as_str()
        .unwrap()
        .contains("repair"));
    assert_eq!(reflection["assignments"][0]["capability_id"], "cap-scan");
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn closing_context_completes_the_root_and_binds_the_summary() {
    let (_config, _home) = isolated_config();
    let client = Arc::new(LayeredClient::new());
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), client.clone()).await;
    let id = "brain-v4-closing";
    create(&node, id).await;
    let first = control::complete_layer(&node, id).await;
    let second = control::complete_layer(&node, id).await;
    assert_eq!(plan::operation(&first[0]).node_id, plan::SCAN);
    assert_eq!(plan::operation(&second[0]).node_id, plan::REVIEW);
    // Every layer is dispatched, so the last context has no nodes and only
    // `complete` or `fail` remain.
    let closing = control::decide_next_layer(&node, id).await;
    assert_eq!(closing.request.plan.nodes.len(), 2, "{closing:?}");
    assert_eq!(closing.layer, plan::total_layers(id));
    let completed = control::wait_phase(&node, id, LayeredPhase::Completed).await;
    assert_eq!(completed.run.layer, 2);
    assert_eq!(
        completed.run.summary.as_deref(),
        Some("layered run complete")
    );
    assert_eq!(client.decisions(), 3, "{:?}", client.instructions());
    // The root itself is finalized: the run result is what a parent plan binds.
    let detail = settled(&node, id).await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");
    assert_eq!(detail["result"]["phase"], "completed", "{detail}");
    assert_eq!(detail["result"]["scheduler_output"], "layered run complete");
    let types = control::event_types(&control::events(&node, id).await);
    assert!(types.contains(&"run_completed".into()), "{types:?}");
    // A terminal run publishes no further delivery.
    let frames = control::frames(&node).await;
    assert!(
        control::actions(&frames, "layered_wake").is_empty(),
        "{frames:?}"
    );
    assert!(
        control::actions(&frames, "layered_dispatch").is_empty(),
        "{frames:?}"
    );
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn historical_schema_four_is_read_only_after_reopening_a_terminal_journal() {
    let (_config, _home) = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), mock()).await;
    let id = "brain-historical";
    create(&node, id).await;
    control::ok(&node, id, "cancel", Value::Null).await;
    node.shutdown().await.unwrap();
    drop(node);
    let path = dir.path().join(format!("node/brain/{id}/execution.json"));
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["assignment"]["request"]["input"] = json!({"schema_version":4,"layered_request":{"schema_version":4,"plan":{"schema_version":4,"title":"historical","objective":"historical","nodes":[{"node_id":"old","title":"old","capability_id":"agent"}]}}});
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let node = worker(dir.path(), mock()).await;
    let before = std::fs::read(&path).unwrap();
    let snapshot = control::ok(&node, id, "snapshot", Value::Null).await;
    assert_eq!(snapshot["schema_version"], 4);
    assert_eq!(snapshot["run"]["phase"], "cancelled");
    assert!(!control::ok(&node, id, "events", json!({})).await["events"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        control::rpc(&node, root(id), "resume", Value::Null)
            .await
            .status,
        409
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "history reads do not rewrite data"
    );
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn exhausted_budget_blocks_dispatch_until_the_budget_command_and_resume() {
    let (_config, _home) = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), Arc::new(LayeredClient::new())).await;
    let id = "brain-round-budget";
    create(&node, id).await;
    for round in 1..=4 {
        control::decide_next_layer(&node, id).await;
        let frame = control::wait_dispatch(&node, 1).await.remove(0);
        let op = plan::operation(&frame);
        assert_eq!(op.round, round);
        control::admit(&node, id, &op.operation_id).await;
        control::ack_dispatch(&node, id, &op.operation_id).await;
        control::finish(&node, id, &frame, LayeredOperationStatus::Error).await;
    }
    control::decide_next_layer(&node, id).await;
    let blocked = control::wait_phase(&node, id, LayeredPhase::Blocked).await;
    assert_eq!(blocked.operations.len(), 4);
    assert!(control::actions(&control::frames(&node).await, "layered_dispatch").is_empty());
    assert!(
        control::rpc(&node, root(id), "set_round_budget", json!({"max_rounds":4}))
            .await
            .status
            >= 300
    );
    control::ok(&node, id, "set_round_budget", json!({"max_rounds":5})).await;
    control::ok(&node, id, "resume", Value::Null).await;
    control::decide_next_layer(&node, id).await;
    let frame = control::wait_dispatch(&node, 1).await.remove(0);
    let op = plan::operation(&frame);
    assert_eq!(op.round, 5);
    control::admit(&node, id, &op.operation_id).await;
    control::ack_dispatch(&node, id, &op.operation_id).await;
    control::finish(&node, id, &frame, LayeredOperationStatus::Done).await;
    control::complete_layer(&node, id).await;
    control::decide_next_layer(&node, id).await;
    let completed = control::wait_phase(&node, id, LayeredPhase::Completed).await;
    assert_eq!(completed.run.max_rounds, 5);
    assert_eq!(completed.run.round, 5);
    assert_eq!(completed.operations.len(), 6);
    node.shutdown().await.unwrap();
}
