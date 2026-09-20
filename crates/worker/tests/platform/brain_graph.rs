use super::*;
use opencoder_core::fleet::*;
use std::sync::Arc;

#[path = "../support/scheduler.rs"]
mod scheduler_support;
use scheduler_support::SchedulerClient;

#[tokio::test]
async fn legacy_brain_planning_is_read_only_and_never_creates_execution() {
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    let plan = opencoder_store::BrainPlanRecord {
        id: "brain-plan-history".into(),
        situation: "review".into(),
        situation_digest: "test".into(),
        chat_model: "mock".into(),
        tree_json:
            json!({"threshold":0.5,"root":{"kind":"leaf","id":"root","capability_id":"old"}})
                .to_string(),
        created_at: 1,
    };
    fleet.state.store.save_brain_plan(&plan).await.unwrap();
    for path in [
        "/api/brain/plans",
        "/api/brain/preview",
        "/api/brain/dispatch",
    ] {
        let reply = fleet
            .call(
                "POST",
                path,
                json!({"situation":"review","plan_id":plan.id,"request_id":"legacy-request"}),
            )
            .await;
        assert_eq!(reply.status, 409, "{reply:?}");
        assert!(reply.body.to_string().contains("migration required"));
    }
    let history = fleet
        .call("GET", &format!("/api/brain/plans/{}", plan.id), Value::Null)
        .await;
    assert_eq!(history.status, 200, "{history:?}");
    assert!(history.body.to_string().contains("brain-plan-history"));
    assert!(fleet
        .state
        .fleet
        .indexes(None, None, 100)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(client.call_count(), 0);
    fleet.shutdown().await;
}

#[tokio::test]
async fn concurrent_v3_runs_generate_once_and_replay_without_new_execution() {
    let client = Arc::new(SchedulerClient::default());
    let fleet = Fleet::new(1, client.clone()).await;
    let body = scheduler_support::request("brain-concurrent");
    let (left, right) = tokio::join!(
        fleet.call("POST", "/api/brain/runs", body.clone()),
        fleet.call("POST", "/api/brain/runs", body.clone())
    );
    assert_eq!(left.status, 202, "{left:?}");
    assert_eq!(right.status, 202, "{right:?}");
    assert_eq!(left.body["run_id"], "brain-concurrent");
    assert_eq!(right.body["run_id"], "brain-concurrent");
    // The child terminal rides the ack-gated outbox replay; a 30s budget has
    // been observed to expire under load spikes even though the delivery
    // chain self-heals, so allow a wider convergence window here.
    scheduler_support::wait_phase_within(&fleet, "brain-concurrent", "completed", 120).await;
    let requests = client.requests.lock().unwrap().clone();
    assert_eq!(
        requests
            .iter()
            .filter(|r| {
                r.purpose == opencoder_llm::RequestPurpose::Planning
                    && serde_json::from_str::<Value>(&r.messages.last().unwrap().text()).unwrap()
                        ["round"]
                        == 0
            })
            .count(),
        1,
        "concurrent callers must share one initial scheduler decision"
    );
    let indexes = fleet.state.fleet.indexes(None, None, 100).await.unwrap();
    assert_eq!(
        indexes
            .iter()
            .filter(|i| i.kind == ExecutionKind::Brain)
            .count(),
        1
    );
    assert_eq!(
        indexes
            .iter()
            .filter(|i| i.kind == ExecutionKind::Agent)
            .count(),
        1
    );
    assert_eq!(
        fleet
            .call("POST", "/api/brain/runs", body.clone())
            .await
            .status,
        202
    );
    assert_eq!(client.requests.lock().unwrap().len(), requests.len());
    fleet.disconnect(0).await;
    let offline_replay = fleet.call("POST", "/api/brain/runs", body.clone()).await;
    assert_eq!(offline_replay.status, 202, "{offline_replay:?}");
    assert_eq!(offline_replay.body, left.body);
    assert_eq!(client.requests.lock().unwrap().len(), requests.len());
    let mut changed = body;
    changed["objective"] = json!("Different requirement");
    assert_eq!(
        fleet.call("POST", "/api/brain/runs", changed).await.status,
        409
    );
    assert_eq!(client.requests.lock().unwrap().len(), requests.len());
    fleet.shutdown().await;
}

#[tokio::test]
async fn an_unconfirmed_v3_run_never_moves_to_another_node_or_plans_offline() {
    let client = Arc::new(SchedulerClient::default());
    let fleet = Fleet::new(2, client.clone()).await;
    let owner = fleet.nodes[0].registration().id;
    fleet
        .state
        .fleet
        .put_index(&ExecutionIndex {
            id: "brain-unconfirmed".into(),
            created_at: 1,
            kind: ExecutionKind::Brain,
            node_id: owner.clone(),
            status: ExecutionStatus::Pending,
        })
        .await
        .unwrap();
    fleet.disconnect(0).await;
    let body = scheduler_support::request("brain-unconfirmed");
    for _ in 0..2 {
        let reply = fleet.call("POST", "/api/brain/runs", body.clone()).await;
        assert_eq!(reply.status, 503, "{reply:?}");
    }
    let index = fleet
        .state
        .fleet
        .index("brain-unconfirmed")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(index.node_id, owner);
    assert!(fleet.nodes[1]
        .indexes()
        .await
        .unwrap()
        .iter()
        .all(|i| i.id != "brain-unconfirmed"));
    assert!(client.requests.lock().unwrap().is_empty());
    fleet.shutdown().await;
}
