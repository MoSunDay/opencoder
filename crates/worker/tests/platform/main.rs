#[path = "../support/mod.rs"]
mod support;
use opencoder_llm::LlmEvent;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use support::*;

#[tokio::test]
async fn brain_preview_binding_and_dispatch_are_idempotent() {
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    let created=fleet.call("POST","/api/brain/capabilities",json!({"capability_type":"agent","summary":"review code","input_desc":"code task","output_desc":"review","eng_inputs":["review code"]})).await;
    assert!(created.status < 300, "{:?}", created);
    let cap = created.body["capability"]["id"].as_str().unwrap();
    let target = fleet
        .call(
            "PUT",
            &format!("/api/brain/capabilities/{cap}/target"),
            json!({"kind":"agent","target":"act"}),
        )
        .await;
    assert_eq!(target.status, 200);
    let plan = opencoder_store::BrainPlanRecord {
        id: "brain-plan-test".into(),
        situation: "review".into(),
        situation_digest: "test".into(),
        chat_model: "mock".into(),
        tree_json: json!({"threshold":0.5,"root":{"kind":"leaf","id":"root","capability_id":cap}})
            .to_string(),
        created_at: 1,
    };
    fleet.state.store.save_brain_plan(&plan).await.unwrap();
    let body = json!({"situation":"review","plan_id":plan.id,"request_id":"brain-request"});
    assert_eq!(
        fleet
            .call("POST", "/api/brain/preview", body.clone())
            .await
            .status,
        200
    );
    assert!(fleet
        .state
        .fleet
        .indexes(None, None, 100)
        .await
        .unwrap()
        .is_empty());
    let dispatched = fleet
        .call("POST", "/api/brain/dispatch", body.clone())
        .await;
    assert_eq!(dispatched.status, 202, "{:?}", dispatched);
    let id = dispatched.body["execution"]["id"].as_str().unwrap();
    assert_eq!(id, "agent-brain-request");
    let accepted = fleet.nodes[0]
        .handle(opencoder_core::fleet::NodeOperation::AcceptedRequest {
            execution: opencoder_core::fleet::ExecutionRef {
                id: id.into(),
                kind: opencoder_core::fleet::ExecutionKind::Agent,
            },
        })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    assert_eq!(accepted.body["receipt"]["intent"]["situation"], "review");
    assert!(accepted.body.get("request").is_none());
    let _ = settled(&fleet.nodes[0], id).await;
    let calls = client.call_count();
    assert_eq!(
        fleet
            .call("POST", "/api/brain/dispatch", body.clone())
            .await
            .status,
        202
    );
    assert_eq!(client.call_count(), calls);
    for changed in [
        json!({"request_id":"brain-request","situation":"changed","plan_id":"brain-plan-test"}),
        json!({"request_id":"brain-request","situation":"review","plan_id":"other-plan"}),
        json!({"request_id":"brain-request","situation":"review","plan_id":"brain-plan-test","top_k":11}),
        json!({"request_id":"brain-request","situation":"review","plan_id":"brain-plan-test","replan":true}),
        json!({"request_id":"brain-request","situation":"review","plan_id":"brain-plan-test","model":"other"}),
        json!({"request_id":"brain-request","situation":"review","plan_id":"brain-plan-test","node_id":fleet.nodes[0].registration().id}),
    ] {
        assert_eq!(
            fleet
                .call("POST", "/api/brain/dispatch", changed)
                .await
                .status,
            409
        );
    }
    assert_eq!(
        fleet
            .call(
                "POST",
                "/api/brain/dispatch",
                json!({"request_id":"brain-request","id":"agent-custom","situation":"review"})
            )
            .await
            .status,
        400
    );
    let foreign = assignment(
        &fleet.nodes[0],
        "agent-foreign",
        opencoder_core::fleet::ExecutionKind::Agent,
        json!({"prompt":"not a brain request"}),
        None,
    );
    let accepted = fleet.nodes[0]
        .handle(opencoder_core::fleet::NodeOperation::Create {
            assignment: foreign,
        })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    fleet
        .state
        .fleet
        .put_index(&serde_json::from_value(accepted.body).unwrap())
        .await
        .unwrap();
    assert_eq!(
        fleet
            .call(
                "POST",
                "/api/brain/dispatch",
                json!({"request_id":"foreign","situation":"work"})
            )
            .await
            .status,
        409
    );
    fleet
        .state
        .fleet
        .put_index(&opencoder_core::fleet::ExecutionIndex {
            id: "team-brain-request".into(),
            created_at: 2,
            kind: opencoder_core::fleet::ExecutionKind::Team,
            node_id: fleet.nodes[0].registration().id,
            status: opencoder_core::fleet::ExecutionStatus::Error,
        })
        .await
        .unwrap();
    assert_eq!(
        fleet.call("POST", "/api/brain/dispatch", body).await.status,
        409
    );
    assert!(fleet.state.store.get_session(id).await.unwrap().is_none());
    fleet.shutdown().await;
}

#[tokio::test]
async fn concurrent_brain_request_plans_once_and_creates_one_execution() {
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    let created = fleet
        .call(
            "POST",
            "/api/brain/capabilities",
            json!({
                "capability_type":"agent","summary":"review code","input_desc":"task",
                "output_desc":"answer","eng_inputs":["review code"]
            }),
        )
        .await;
    let cap = created.body["capability"]["id"].as_str().unwrap();
    assert_eq!(
        fleet
            .call(
                "PUT",
                &format!("/api/brain/capabilities/{cap}/target"),
                json!({"kind":"agent","target":"act"})
            )
            .await
            .status,
        200
    );
    client.queue_script(vec![LlmEvent::Completed {
        text: json!({"threshold":0.5,"root":{
            "kind":"leaf","id":"root","capability_id":cap,"reason":"review"
        }})
        .to_string(),
        tool_calls: vec![],
        usage: None,
    }]);
    let before = client.call_count();
    let body = json!({"situation":"review code","request_id":"concurrent","replan":true});
    let (left, right) = tokio::join!(
        fleet.call("POST", "/api/brain/dispatch", body.clone()),
        fleet.call("POST", "/api/brain/dispatch", body)
    );
    assert_eq!(left.status, 202, "{left:?}");
    assert_eq!(right.status, 202, "{right:?}");
    assert_eq!(left.body["execution"]["id"], "agent-concurrent");
    assert_eq!(right.body["execution"]["id"], "agent-concurrent");
    let _ = settled(&fleet.nodes[0], "agent-concurrent").await;
    assert_eq!(
        client.requests()[before..]
            .iter()
            .filter(|request| {
                request.messages.iter().any(|message| {
                    message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("能力动态规划器"))
                })
            })
            .count(),
        1,
        "concurrent callers must share one planner decision"
    );
    assert_eq!(
        fleet
            .state
            .fleet
            .indexes(None, None, 100)
            .await
            .unwrap()
            .iter()
            .filter(|index| index.id.ends_with("-concurrent"))
            .count(),
        1
    );
    fleet.shutdown().await;
}

#[tokio::test]
async fn existing_brain_candidate_never_replans_or_moves_nodes() {
    let client = mock();
    let fleet = Fleet::new(2, client.clone()).await;
    let owner = fleet.nodes[0].registration().id;
    fleet
        .state
        .fleet
        .put_index(&opencoder_core::fleet::ExecutionIndex {
            id: "agent-unconfirmed".into(),
            created_at: 1,
            kind: opencoder_core::fleet::ExecutionKind::Agent,
            node_id: owner.clone(),
            status: opencoder_core::fleet::ExecutionStatus::Pending,
        })
        .await
        .unwrap();
    let body = json!({"situation":"work","request_id":"unconfirmed"});
    assert_eq!(
        fleet
            .call("POST", "/api/brain/dispatch", body.clone())
            .await
            .status,
        503
    );
    assert_eq!(client.call_count(), 0);
    fleet.disconnect(0).await;
    assert_eq!(
        fleet.call("POST", "/api/brain/dispatch", body).await.status,
        503
    );
    let index = fleet
        .state
        .fleet
        .index("agent-unconfirmed")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(index.node_id, owner);
    assert!(fleet.nodes[1]
        .indexes()
        .await
        .unwrap()
        .iter()
        .all(|index| index.id != "agent-unconfirmed"));
    assert_eq!(client.call_count(), 0);
    fleet.shutdown().await;
}

mod workloads;
