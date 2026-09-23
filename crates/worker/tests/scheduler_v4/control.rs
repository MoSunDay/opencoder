#![allow(dead_code)]
//! Control-side simulation for the node-level v4 tests.
//!
//! Every helper mirrors one control call or one delivery decision: control owns
//! no scheduler state, so a root can be driven end to end from a test.
use crate::plan;
use opencoder_core::{brain::layered::*, fleet::*};
use opencoder_node::fleet::NodeService;
use opencoder_worker::Worker;
use serde_json::{json, Value};
use std::time::Duration;

pub const TIMEOUT: Duration = Duration::from_secs(30);

pub async fn rpc(node: &Worker, reference: ExecutionRef, action: &str, input: Value) -> RpcReply {
    node.handle(NodeOperation::Brain {
        execution: reference,
        action: action.into(),
        input,
    })
    .await
}

/// One accepted root call; a rejected call is a test failure, not a state.
pub async fn ok(node: &Worker, id: &str, action: &str, input: Value) -> Value {
    let reply = rpc(
        node,
        ExecutionRef {
            id: id.into(),
            kind: ExecutionKind::Brain,
        },
        action,
        input,
    )
    .await;
    assert_eq!(reply.status, 200, "{action}: {reply:?}");
    reply.body
}

/// Every brain frame the node currently publishes; frames replay until the
/// acknowledgement gate closes them.
pub async fn frames(node: &Worker) -> Vec<NodeFrame> {
    node.brain_frames().await.unwrap()
}

/// Every frame of one action, in delivery order.
pub fn actions(frames: &[NodeFrame], action: &str) -> Vec<Value> {
    frames
        .iter()
        .filter_map(|frame| match frame {
            NodeFrame::Brain {
                action: current,
                input,
                ..
            } if current == action => Some(input.clone()),
            _ => None,
        })
        .collect()
}

/// Poll until the node publishes at least one frame of `action` that matches,
/// then return every matching frame.
pub async fn wait_frames(
    node: &Worker,
    action: &str,
    predicate: impl Fn(&Value) -> bool,
) -> Vec<Value> {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let found: Vec<Value> = actions(&frames(node).await, action)
                .into_iter()
                .filter(|frame| predicate(frame))
                .collect();
            if !found.is_empty() {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for a {action} frame"))
}

pub async fn wait_wake(node: &Worker) -> Value {
    wait_frames(node, "layered_wake", |_| true).await.remove(0)
}

/// The published dispatches of one layer. The node only publishes an operation
/// after the finite decision that created it is durable.
pub async fn wait_dispatch(node: &Worker, layer: u32) -> Vec<Value> {
    wait_frames(node, "layered_dispatch", |frame| {
        frame["operation"]["layer"] == json!(layer)
    })
    .await
}

pub async fn snapshot(node: &Worker, id: &str) -> LayeredSnapshot {
    serde_json::from_value(ok(node, id, "snapshot", json!({})).await).expect("layered snapshot")
}

pub async fn wait_phase(node: &Worker, id: &str, phase: LayeredPhase) -> LayeredSnapshot {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let current = snapshot(node, id).await;
            if current.run.phase == phase {
                return current;
            }
            assert_ne!(
                current.run.phase,
                LayeredPhase::Blocked,
                "layered run blocked: {current:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("layered phase timeout")
}

/// Poll the projection until an operation appears; retries are published
/// without a new wake, so the operation index is the only signal.
pub async fn wait_operation(node: &Worker, id: &str, operation_id: &str) -> LayeredOperation {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let current = snapshot(node, id).await;
            if let Some(operation) = current
                .operations
                .iter()
                .find(|operation| operation.operation_id == operation_id)
            {
                return operation.clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for operation {operation_id}"))
}

/// Acknowledge the visible wake, then install the context of the layer the run
/// decides next. The closing case hands over the empty canvas control builds
/// once every layer is dispatched.
pub async fn decide_next_layer(node: &Worker, id: &str) -> LayeredContext {
    let generation = wait_wake(node).await["generation"]
        .as_u64()
        .expect("wake generation");
    ok(
        node,
        id,
        "layered_wake_ack",
        json!({"generation": generation}),
    )
    .await;
    let context = plan::next_context(id, &snapshot(node, id).await);
    let installed = ok(node, id, "layered_context", json!(context)).await;
    assert_ne!(installed["stale"], true, "stale layer context: {installed}");
    context
}

pub async fn authorize(node: &Worker, id: &str, frame: &Value) -> Value {
    let body = ok(node, id, "layered_authorize", frame.clone()).await;
    assert_eq!(body["authorized"], true, "{body}");
    body
}

/// The child was created: the node admits the operation and keeps the layer
/// barrier open until the child reports its terminal.
pub async fn admit(node: &Worker, id: &str, operation_id: &str) -> Value {
    let body = ok(
        node,
        id,
        "layered_receipt",
        json!({"operation_id": operation_id, "reply": {"status": 200, "body": {"accepted": true}}}),
    )
    .await;
    assert_ne!(body["duplicate"], true, "{body}");
    body
}

/// Fold the terminal a settled child reports for one dispatch.
pub async fn finish(
    node: &Worker,
    id: &str,
    frame: &Value,
    status: LayeredOperationStatus,
) -> LayeredTerminalEvent {
    let notice = plan::terminal_notice(&plan::operation(frame), status, 7);
    let body = ok(node, id, "layered_terminal", json!(notice)).await;
    assert_ne!(body["duplicate"], true, "{body}");
    notice
}

pub async fn ack_dispatch(node: &Worker, id: &str, operation_id: &str) -> Value {
    ok(
        node,
        id,
        "layered_dispatch_ack",
        json!({"operation_id": operation_id}),
    )
    .await
}

/// One full control round trip for one layer: every node of the layer is
/// created, admitted and reported successful.
pub async fn complete_layer(node: &Worker, id: &str) -> Vec<Value> {
    let context = decide_next_layer(node, id).await;
    assert!(
        !context.request.plan.nodes.is_empty(),
        "expected a layer context"
    );
    let dispatches = wait_dispatch(node, context.layer + 1).await;
    assert_eq!(
        dispatches.len(),
        context
            .request
            .plan
            .nodes
            .iter()
            .filter(|n| n.layer == context.layer + 1)
            .count(),
        "{dispatches:?}"
    );
    for frame in &dispatches {
        authorize(node, id, frame).await;
        let operation = plan::operation(frame);
        admit(node, id, &operation.operation_id).await;
        ack_dispatch(node, id, &operation.operation_id).await;
        finish(node, id, frame, LayeredOperationStatus::Done).await;
    }
    dispatches
}

pub async fn events(node: &Worker, id: &str) -> Vec<Value> {
    ok(node, id, "events", json!({"after": 0, "limit": 100})).await["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

pub fn event_types(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| event["event_type"].as_str().map(str::to_string))
        .collect()
}
