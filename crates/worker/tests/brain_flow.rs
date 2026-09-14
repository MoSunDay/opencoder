mod support;
use opencoder_core::{brain::ActionReceipt, fleet::*};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use support::*;

async fn snapshot(node: &opencoder_worker::Worker, reference: &ExecutionRef) -> Value {
    node.handle(NodeOperation::Brain {
        execution: reference.clone(),
        action: "snapshot".into(),
        input: json!({}),
    })
    .await
    .body
}
async fn next_action(node: &opencoder_worker::Worker, seen: &[String]) -> ActionReceipt {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            for frame in node.brain_frames().await.unwrap() {
                if let NodeFrame::Brain { action, input, .. } = frame {
                    if action == "action" {
                        let receipt: ActionReceipt = serde_json::from_value(input).unwrap();
                        if !seen.contains(&receipt.id) {
                            return receipt;
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("next flow action")
}

#[tokio::test]
async fn repair_flow_persists_visits_feedback_and_release_gate_on_node() {
    let _config = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), mock()).await;
    let reference = ExecutionRef {
        id: "brain-flow".into(),
        kind: ExecutionKind::Brain,
    };
    let plan: Value =
        serde_json::from_str(include_str!("../../../examples/brain/repair-loop.json")).unwrap();
    let version = json!({"id":"repair-loop","version":1,"plan":plan,"changelog":"fixture","created_at":1,"author":"test"});
    let reply = node.handle(NodeOperation::Create { assignment: assignment(&node, &reference.id, reference.kind,
        json!({"mode":"fixed","objective":"repair and verify","plan":version,"inputs":{"problem":"repro"}}), Some(json!({}))) }).await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let mut seen = vec![];
    for (expected, output) in [
        ("fix", json!({"commit":"fix-1"})),
        ("verify", json!({"passed":false,"issues":"still broken"})),
        ("fix", json!({"commit":"fix-2"})),
        ("verify", json!({"passed":true,"issues":""})),
        ("release", json!({"result":"released fix-2"})),
    ] {
        let receipt = next_action(&node, &seen).await;
        assert!(receipt
            .instance_id
            .starts_with(&format!("{expected}~visit-")));
        if expected == "release" {
            let instance = node
                .handle(NodeOperation::Brain {
                    execution: reference.clone(),
                    action: "instance".into(),
                    input: json!({"id":receipt.instance_id}),
                })
                .await;
            assert_eq!(instance.body["inputs"]["commit"], "fix-2");
            assert_eq!(instance.body["inputs"]["verified"], true);
        }
        if seen.len() == 2 {
            let instance = node
                .handle(NodeOperation::Brain {
                    execution: reference.clone(),
                    action: "instance".into(),
                    input: json!({"id":receipt.instance_id}),
                })
                .await;
            assert_eq!(instance.body["inputs"]["feedback"], "still broken");
            assert!(!snapshot(&node, &reference).await["instances"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["step_id"] == "release"));
        }
        let notice = json!({"parent":receipt.request["input"]["_brain"]["parent"],"execution":{"id":receipt.id,"kind":"agent"},"node_id":"child-node","sequence":1,"status":"succeeded","output":{"value":output,"artifacts":[],"evidence":["fixture"]},"error":null,"at_ms":5});
        seen.push(receipt.id);
        let reply = node
            .handle(NodeOperation::Brain {
                execution: reference.clone(),
                action: "notice".into(),
                input: notice.clone(),
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        let duplicate = node
            .handle(NodeOperation::Brain {
                execution: reference.clone(),
                action: "notice".into(),
                input: notice,
            })
            .await;
        assert_eq!(duplicate.body["duplicate"], true);
    }
    let state = snapshot(&node, &reference).await;
    assert_eq!(state["phase"], "completed");
    assert_eq!(state["total_instances"], 5);
    assert_eq!(state["deliverables"]["release_result"], "released fix-2");
    node.shutdown().await.unwrap();
}
