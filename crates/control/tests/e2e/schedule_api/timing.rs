use super::*;
use opencoder_store::SCHEDULE_RUN_ERROR;

#[tokio::test]
async fn unchanged_updates_preserve_the_scheduling_baseline() {
    let h = Harness::new().await;
    let body = json!({"id": "unchanged", "cron": "0 0 * * *", "kind": "agent",
        "target": "act", "params": {"prompt": "ping"}, "enabled": false});
    let (status, reply) = create_schedule(&h, body.clone()).await;
    assert_eq!(status, 200, "{reply}");
    let original = h
        .state
        .store
        .get_schedule("unchanged")
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    for (method, payload) in [
        (reqwest::Method::PUT, body),
        (reqwest::Method::PATCH, json!({"enabled": false})),
    ] {
        let (status, reply) = h
            .req(method, "/api/schedules/unchanged", Some(payload))
            .await;
        assert_eq!(status, 200, "{reply}");
        let current = h
            .state
            .store
            .get_schedule("unchanged")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.updated_at, original.updated_at);
    }
}

/// Scheduling starts with the current definition, never with a fabricated
/// 24h history. Exercise creation, re-enabling and editing through HTTP.
#[tokio::test]
async fn definition_changes_do_not_backfill_or_retry_old_ticks() {
    let h = Harness::new().await;
    write_fast_scan(&h);
    let now = opencoder_core::message::now_ms();
    let next = opencoder_core::schedule::to_utc(now + 6 * 60 * 60 * 1000);
    let cron = format!("{} * * *", next.format("%M %H"));
    let body = |id, enabled| {
        json!({
            "id": id, "cron": cron, "kind": "agent", "target": "act",
            "params": {"prompt": "ping"}, "enabled": enabled,
        })
    };
    let (status, created) = create_schedule(&h, body("new_daily", true)).await;
    assert_eq!(status, 200, "{created}");

    for id in ["resumed_daily", "edited_daily"] {
        // Simulate a definition and failed fire from an earlier lifetime.
        let job = serde_json::from_value(body(id, false)).unwrap();
        h.state
            .store
            .upsert_schedule(&job, now - 2 * 86_400_000)
            .await
            .unwrap();
        let mut failed = fired_run(id, now - 60_000, "unused");
        failed.status = SCHEDULE_RUN_ERROR.into();
        failed.execution_id = None;
        failed.error = Some("previous definition failed".into());
        h.state.store.record_schedule_run(&failed).await.unwrap();
        let (method, payload) = if id == "resumed_daily" {
            (reqwest::Method::PATCH, json!({"enabled": true}))
        } else {
            (reqwest::Method::PUT, body(id, true))
        };
        let (status, reply) = h
            .req(method, &format!("/api/schedules/{id}"), Some(payload))
            .await;
        assert_eq!(status, 200, "{reply}");
    }

    let before = h.state.lifecycle.scheduler.snapshot().scans_total;
    tokio::time::timeout(Duration::from_secs(45), async {
        while h.state.lifecycle.scheduler.snapshot().scans_total < before + 2 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("scheduler must scan the stored definitions");

    assert_eq!(
        h.state.lifecycle.scheduler.snapshot().fire_attempts_total,
        0,
        "no historical dispatch or retry before the next cron tick"
    );
    assert!(runs_of(&h, "new_daily").await["runs"]
        .as_array()
        .unwrap()
        .is_empty());
    for id in ["resumed_daily", "edited_daily"] {
        let history = runs_of(&h, id).await;
        let runs = history["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 1, "only the original error remains: {history}");
        assert_eq!(runs[0]["status"], "error");
        assert_eq!(runs[0]["fired_at_ms"], now - 60_000);
    }
}
