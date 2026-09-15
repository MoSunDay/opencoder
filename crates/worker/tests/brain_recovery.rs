mod support;
use opencoder_core::{brain::*, fleet::*};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use support::*;

fn plan() -> Value {
    json!({"id":"plan-recovery","version":1,"changelog":"initial","created_at":1,"author":"test","plan":{"title":"Recovery","objective":"finish once","steps":[{"id":"one","label":"One","purpose":"Return evidence","action":{"kind":"agent","target":"act","prompt":"Return result"},"output":{"type":"string"},"acceptance":"result exists"}],"deliverables":{"result":{"description":"result","source":{"source":"output","step":"one"},"schema":{"type":"string"}}}}})
}
async fn snapshot(node: &opencoder_worker::Worker, reference: &ExecutionRef) -> Value {
    node.handle(NodeOperation::Brain {
        execution: reference.clone(),
        action: "snapshot".into(),
        input: json!({}),
    })
    .await
    .body
}

#[tokio::test]
async fn prepared_action_replays_after_restart_and_duplicate_notice_keeps_watermark() {
    let _config = support::isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let client = mock();
    let node = worker(dir.path(), client.clone()).await;
    let reference = ExecutionRef {
        id: "brain-recovery".into(),
        kind: ExecutionKind::Brain,
    };
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                &reference.id,
                reference.kind,
                json!({"mode":"fixed","objective":"finish once","plan":plan()}),
                Some(json!({})),
            ),
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    settled(&node, &reference.id).await;
    let before = snapshot(&node, &reference).await;
    let frames = node.brain_frames().await.unwrap();
    let action = frames
        .into_iter()
        .find_map(|f| match f {
            NodeFrame::Brain { action, input, .. } if action == "action" => Some(input),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        client.call_count(),
        0,
        "fixed activation does not call the model"
    );
    node.shutdown().await.unwrap();
    drop(node);
    let node = worker(dir.path(), client).await;
    let replay = node
        .brain_frames()
        .await
        .unwrap()
        .into_iter()
        .find_map(|f| match f {
            NodeFrame::Brain { action, input, .. } if action == "action" => Some(input),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        action, replay,
        "uncertain dispatch keeps the same action and request"
    );
    assert_eq!(before["plan"], snapshot(&node, &reference).await["plan"]);
    let receipt: ActionReceipt = serde_json::from_value(action).unwrap();
    let call = |action: &str, input: Value| {
        node.handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: action.into(),
            input,
        })
    };
    // A lost dispatch reply may update the retry error while old frames remain in flight.
    assert_eq!(
        call(
            "receipt",
            json!({"id":receipt.id,"reply":RpcReply::error(503,"uncertain transport")})
        )
        .await
        .status,
        200
    );
    assert_eq!(call("authorize", json!(receipt)).await.status, 200);
    let mut changed = receipt.clone();
    changed.request["input"]["prompt"] = json!("changed request");
    assert_eq!(call("authorize", json!(changed)).await.status, 409);
    assert_eq!(
        call(
            "receipt",
            json!({"id":receipt.id,"reply":RpcReply::ok(json!({"node_id":"child-node"}))})
        )
        .await
        .status,
        200
    );
    assert_eq!(call("authorize", json!(receipt)).await.status, 409);
    let notice = json!({"parent":receipt.request["input"]["_brain"]["parent"],"execution":{"id":receipt.id,"kind":"agent"},"node_id":"child-node","sequence":3,"status":"succeeded","output":{"value":"verified","artifacts":[],"evidence":["receipt"]},"error":null,"at_ms":5});
    let first = node
        .handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: "notice".into(),
            input: notice.clone(),
        })
        .await;
    assert_eq!(first.status, 200, "{first:?}");
    let completed = snapshot(&node, &reference).await;
    assert_eq!(completed["phase"], "completed");
    assert_eq!(completed["deliverables"]["result"], "verified");
    assert_eq!(
        call("published", completed["plan"].clone()).await.body["duplicate"],
        true
    );
    assert_eq!(
        node.handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: "notice".into(),
            input: notice
        })
        .await
        .body["duplicate"],
        true
    );
    assert_eq!(
        snapshot(&node, &reference).await["watermark"],
        completed["watermark"]
    );
    assert!(node.brain_frames().await.unwrap().is_empty());
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn planning_can_pause_before_activation_and_cancel_without_a_plan() {
    let _config = support::isolated_config();
    let request = BrainRequest {
        mode: PlanningMode::Dynamic,
        objective: "plan later".into(),
        inputs: Default::default(),
        plan: None,
        references: vec![],
        capabilities: vec![],
    };
    let mut run = opencoder_brain::execution::initialize("brain-early", request, 1).unwrap();
    opencoder_brain::execution::command(&mut run, "pause", 2).unwrap();
    assert_eq!(run.phase, RunPhase::Paused);
    opencoder_brain::execution::command(&mut run, "cancel", 3).unwrap();
    assert_eq!(run.phase, RunPhase::Cancelled);
}
