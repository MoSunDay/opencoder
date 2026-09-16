#[path = "support/brain.rs"]
mod graph_support;
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
    let client = mock();
    let node = worker(dir.path(), client.clone()).await;
    let reference = ExecutionRef {
        id: "brain-flow".into(),
        kind: ExecutionKind::Brain,
    };
    let plan: Value =
        serde_json::from_str(include_str!("../../../examples/brain/repair-loop.json")).unwrap();
    let version = json!({"id":"repair-loop","version":1,"plan":plan,"changelog":"fixture","created_at":1,"author":"test"});
    let reply = node.handle(NodeOperation::Create { assignment: assignment(&node, &reference.id, reference.kind,
        json!({"schema_version":2,"mode":"fixed","objective":"repair and verify","plan":version,"inputs":{"document":{"name":"Problem","markdown":"repro"}}}), Some(json!({}))) }).await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let mut seen = vec![];
    for (expected, name, content, passed, next) in [
        ("fix", "fix-result", "fix-1", false, Some("verify")),
        ("verify", "verification", "still broken", false, Some("fix")),
        ("fix", "fix-result", "fix-2", false, Some("verify")),
        ("verify", "verification", "verified fix-2", true, None),
    ] {
        let output = json!({name: {"content":content,"completion":{"passed":true,"evidence":["delivered"]},"verification":{"passed":passed,"evidence":["tests"]}}});
        let receipt = next_action(&node, &seen).await;
        assert!(receipt
            .instance_id
            .starts_with(&format!("{expected}~visit-")));
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
        let route = if expected == "fix" {
            "after-fix"
        } else {
            "after-verify"
        };
        let key = format!(
            "route-{}",
            &opencoder_brain::execution::fingerprint(&(route, vec![receipt.instance_id.clone()]))
                [..32]
        );
        client.queue_script(vec![opencoder_llm::LlmEvent::Completed {text:json!({"receipt":key,"reason":"fixture output evidence","selected":next.into_iter().collect::<Vec<_>>(),"exit":if next.is_none(){Some("done")}else{None}}).to_string(),tool_calls:vec![],usage:None}]);
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
    let state = graph_support::wait_phase(&node, &reference, "completed").await;
    assert_eq!(state["phase"], "completed");
    assert_eq!(state["total_instances"], 4);
    assert_eq!(
        state["deliverables"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap(),
        "verified fix-2"
    );
    node.shutdown().await.unwrap();
}
