//! Scheduler metrics use the real authenticated router and stored fire ledger.
use crate::support::{http::Harness, TOKEN};
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use opencoder_store::{ScheduleRunRecord, SCHEDULE_RUN_ERROR, SCHEDULE_RUN_FIRED};
use reqwest::Method;
use serde_json::json;

#[tokio::test]
async fn reports_cluster_capacity_and_schedule_outcomes_without_task_labels() {
    let h = Harness::with_metrics_token("metrics-only-test").await;
    for id in ["metrics-active", "metrics-done", "metrics-error"] {
        let (status, body) = h
            .req(
                Method::POST,
                "/api/schedules",
                Some(json!({
                    "id": id, "cron": "0 */2 * * *", "kind": "agent", "target": "act"
                })),
            )
            .await;
        assert_eq!(status, 200, "{body}");
    }
    let now = opencoder_core::message::now_ms();
    let active_id = "agent-metrics-active-run";
    let done_id = "agent-metrics-done-run";
    h.put_index(active_id, ExecutionKind::Agent, ExecutionStatus::Running)
        .await;
    h.put_index(done_id, ExecutionKind::Agent, ExecutionStatus::Done)
        .await;
    for (id, status, execution_id) in [
        ("metrics-active", SCHEDULE_RUN_FIRED, Some(active_id)),
        ("metrics-done", SCHEDULE_RUN_FIRED, Some(done_id)),
        ("metrics-error", SCHEDULE_RUN_ERROR, None),
    ] {
        h.state
            .store
            .record_schedule_run(&ScheduleRunRecord {
                schedule_id: id.into(),
                kind: "agent".into(),
                target: "act".into(),
                scheduled_for_ms: now,
                fired_at_ms: now,
                execution_id: execution_id.map(str::to_string),
                status: status.into(),
                error: (status == SCHEDULE_RUN_ERROR).then(|| "admission failed".into()),
                missed: false,
            })
            .await
            .unwrap();
    }
    let (status, body) = h.req(Method::GET, "/api/metrics/scheduler", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["schedules_total"], 3);
    assert_eq!(body["schedules_enabled"], 3);
    assert_eq!(body["schedules_last_fire_error"], 1);
    assert_eq!(body["schedules_last_execution_active"], 1);
    assert_eq!(body["schedules_last_execution_done"], 1);
    assert!(body["nodes_ready"].as_u64().unwrap() >= 1);
    assert!(body["next_tick_ms"].as_i64().unwrap() > now);

    let response = h.req_raw(Method::GET, "/metrics", None, Some(TOKEN)).await;
    assert_eq!(response.status(), 200);
    assert!(response.headers()[reqwest::header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/plain"));
    let metrics = response.text().await.unwrap();
    assert!(metrics.contains("opencoder_scheduler_schedules_last_fire_error 1\n"));
    assert!(metrics.contains("opencoder_scheduler_schedules_last_execution_active 1\n"));
    assert!(metrics.contains("opencoder_scheduler_schedules_last_execution_done 1\n"));
    assert!(!metrics.contains("metrics-active"));
    assert_eq!(
        h.req_raw(Method::GET, "/metrics", None, None)
            .await
            .status(),
        401
    );
    assert_eq!(
        h.req_raw(Method::GET, "/metrics", None, Some("metrics-only-test"))
            .await
            .status(),
        200
    );
    assert_eq!(
        h.req_raw(
            Method::GET,
            "/api/metrics/scheduler",
            None,
            Some("metrics-only-test")
        )
        .await
        .status(),
        401
    );
    assert_eq!(
        h.req_raw(
            Method::POST,
            "/api/admin/drain",
            None,
            Some("metrics-only-test")
        )
        .await
        .status(),
        401
    );
}
