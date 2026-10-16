use super::*;
use opencoder_store::SCHEDULE_RUN_ERROR;
use reqwest::Method;

async fn failed_fire(h: &Harness, id: &str, status: u16) -> ScheduleRunRecord {
    write_fast_scan(h);
    h.node
        .set_create_reply(status, json!({"error": "preflight unavailable"}));
    let (code, body) = create_schedule(
        h,
        json!({
            "id": id, "cron": USER_AGENT_CRON, "kind": "agent", "target": "act",
            "params": {"prompt": "ping"}
        }),
    )
    .await;
    assert_eq!(code, 200, "{body}");
    let (code, body) = h
        .req(Method::POST, &format!("/api/schedules/{id}/run"), None)
        .await;
    assert!(code.is_server_error(), "{code}: {body}");
    let failed = h.state.store.last_schedule_run(id).await.unwrap().unwrap();
    assert_eq!(failed.status, SCHEDULE_RUN_ERROR);
    assert!(h.node.journal_ids().is_empty());
    h.node.clear_create_reply();
    failed
}

fn tick_id(run: &ScheduleRunRecord) -> String {
    format!("{}-{}-{}", run.kind, run.schedule_id, run.scheduled_for_ms)
}

#[tokio::test]
async fn rejected_tick_preserves_failure_and_allows_the_next_tick() {
    let h = Harness::new().await;
    let failed = failed_fire(&h, "rejected", 400).await;
    let failed_id = tick_id(&failed);
    let receipt = h
        .state
        .fleet
        .receipt("execution", &failed_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.phase, "rejected");

    let body = poll_runs(&h, "rejected", |b| {
        b["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["status"] == "fired")
    })
    .await;
    let runs = body["runs"].as_array().unwrap();
    let next = runs
        .iter()
        .find(|r| r["status"] == "fired")
        .expect("a durable rejection must not block future cron ticks");
    assert!(next["scheduled_for_ms"].as_i64().unwrap() > failed.scheduled_for_ms);
    assert!(h
        .node
        .journal_ids()
        .contains(&next["execution_id"].as_str().unwrap().to_string()));
    assert!(!h.node.journal_ids().contains(&failed_id));
    let original = runs
        .iter()
        .find(|r| r["scheduled_for_ms"] == failed.scheduled_for_ms)
        .unwrap();
    assert_eq!(original, &serde_json::to_value(&failed).unwrap());

    // An explicit replay of the rejected request must still be idempotent.
    let (code, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({
                "id": failed_id, "kind": "agent", "target": "act", "input": {"prompt": "ping"}
            })),
        )
        .await;
    assert_eq!(code, 400, "{body}");
    assert_eq!(body, receipt.payload["body"]);
}

#[tokio::test]
async fn transient_failure_retries_the_same_tick() {
    let h = Harness::new().await;
    let failed = failed_fire(&h, "transient", 503).await;
    let failed_id = tick_id(&failed);
    let body = poll_runs(&h, "transient", |b| {
        b["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["execution_id"] == failed_id && r["status"] == "fired")
    })
    .await;
    let recovered = body["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["execution_id"] == failed_id && r["status"] == "fired")
        .expect("temporary failures must retry the original request");
    assert_eq!(recovered["scheduled_for_ms"], failed.scheduled_for_ms);
    assert!(h.node.journal_ids().contains(&failed_id));
    assert_eq!(
        h.state
            .fleet
            .receipt("execution", &failed_id)
            .await
            .unwrap()
            .unwrap()
            .phase,
        "accepted"
    );
}
