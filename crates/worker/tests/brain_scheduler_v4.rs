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
    let input = json!({"schema_version": 4, "layered_request": plan::request(id)});
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
    assert_eq!(context.layer, 1);
    assert_eq!(context.nodes.len(), 1, "{context:?}");
    // Installing a context only changes the durable projection, so the node
    // must requeue its own activation before one layer can be decided.
    let waiting = control::wait_phase(&node, id, LayeredPhase::Waiting).await;
    assert_eq!(waiting.run.layer, 1, "{waiting:?}");
    assert_eq!(client.decisions(), 1, "{:?}", client.instructions());
    let dispatches = control::wait_dispatch(&node, 1).await;
    assert_eq!(dispatches.len(), 1, "{dispatches:?}");
    let operation = plan::operation(&dispatches[0]);
    assert_eq!(operation.operation_id, format!("{id}#l1#scan#a1"));
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
async fn failed_attempt_retries_then_the_barrier_opens_the_next_layer() {
    let (_config, _home) = isolated_config();
    let client = Arc::new(LayeredClient::new());
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), client.clone()).await;
    let id = "brain-v4-retry";
    create(&node, id).await;
    let context = control::decide_next_layer(&node, id).await;
    assert_eq!(context.layer, 1);
    let first = control::wait_dispatch(&node, 1).await.remove(0);
    let attempt = plan::operation(&first);
    assert_eq!(attempt.attempt, 1);
    control::authorize(&node, id, &first).await;
    control::admit(&node, id, &attempt.operation_id).await;
    control::ack_dispatch(&node, id, &attempt.operation_id).await;
    control::finish(&node, id, &first, LayeredOperationStatus::Error).await;
    // `retry.max_attempts` is 2, so the failed attempt schedules the next one
    // without re-deciding the layer or failing the run.
    let retry_id = format!("{id}#l1#scan#a2");
    let retry = control::wait_operation(&node, id, &retry_id).await;
    assert_eq!(retry.status, LayeredOperationStatus::Creating);
    assert_eq!(retry.attempt, 2);
    assert_ne!(retry.execution_id, attempt.execution_id);
    assert_eq!(
        control::snapshot(&node, id).await.run.phase,
        LayeredPhase::Waiting
    );
    let types = control::event_types(&control::events(&node, id).await);
    assert!(
        types.contains(&"operation_retry_scheduled".into()),
        "{types:?}"
    );
    assert!(!types.contains(&"run_failed".into()), "{types:?}");
    // The retried attempt is dispatched from the same durable intent.
    let retried = control::wait_dispatch(&node, 1).await;
    assert_eq!(retried.len(), 1, "{retried:?}");
    assert_eq!(plan::operation(&retried[0]).operation_id, retry_id);
    control::authorize(&node, id, &retried[0]).await;
    control::admit(&node, id, &retry_id).await;
    control::ack_dispatch(&node, id, &retry_id).await;
    control::finish(&node, id, &retried[0], LayeredOperationStatus::Done).await;
    // Only a successful attempt of every node of the layer opens the barrier.
    let ready = control::wait_phase(&node, id, LayeredPhase::Ready).await;
    assert_eq!(ready.run.layer, 1, "{ready:?}");
    let types = control::event_types(&control::events(&node, id).await);
    assert!(types.contains(&"layer_barrier_reached".into()), "{types:?}");
    // The next context carries the successful attempt as its direct upstream, so
    // layer two can bind an ancestor execution instead of the root input.
    let next = control::decide_next_layer(&node, id).await;
    assert_eq!(next.layer, 2);
    assert_eq!(next.nodes.len(), 1);
    let upstream = &next.nodes[0].upstream[0];
    assert_eq!(upstream.node_id, plan::SCAN);
    assert_eq!(
        upstream.execution_id.as_deref(),
        Some(retry.execution_id.as_str())
    );
    let review = control::wait_dispatch(&node, 2).await;
    assert_eq!(review.len(), 1, "{review:?}");
    let operation = plan::operation(&review[0]);
    assert_eq!(operation.operation_id, format!("{id}#l2#review#a1"));
    assert_eq!(
        review[0]["assignment"]["inputs"]["source"],
        json!({"kind": "execution", "execution_id": retry.execution_id, "path": ""})
    );
    control::authorize(&node, id, &review[0]).await;
    control::admit(&node, id, &operation.operation_id).await;
    control::ack_dispatch(&node, id, &operation.operation_id).await;
    control::finish(&node, id, &review[0], LayeredOperationStatus::Done).await;
    let barrier = control::wait_phase(&node, id, LayeredPhase::Ready).await;
    assert_eq!(barrier.run.layer, 2, "{barrier:?}");
    assert_eq!(
        client.decisions(),
        2,
        "retries never re-decide: {:?}",
        client.instructions()
    );
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
    assert!(closing.nodes.is_empty(), "{closing:?}");
    assert_eq!(closing.layer, plan::total_layers(id) + 1);
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
