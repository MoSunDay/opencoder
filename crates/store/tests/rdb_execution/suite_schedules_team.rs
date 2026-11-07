//! Schedule + team-run contracts, ported from `tests/schedule_defs.rs`,
//! `tests/schedule_runs.rs` and `tests/team_runs.rs`: definition CRUD with
//! server-owned timestamps, the FK-less fire ledger, and the (topic, node)
//! fan-out ledger with its node cascade realized as GC-on-list.

use super::*;
use opencoder_core::config::{ScheduleJob, ScheduleKind, ScheduleOverlap};
use opencoder_store::{
    ScheduleRunRecord, TeamTopicRunRecord, SCHEDULE_RUN_ERROR, SCHEDULE_RUN_FIRED,
    SCHEDULE_RUN_MISSED, TEAM_RUN_EXECUTING, TEAM_RUN_FINISHED,
};
use std::collections::BTreeMap;

fn job(id: &str) -> ScheduleJob {
    let mut params = BTreeMap::new();
    params.insert("prompt".to_string(), serde_json::json!("ping {{now:%Y}}"));
    ScheduleJob {
        id: id.to_string(),
        cron: "0 3 * * *".to_string(),
        timezone: Some("+08:00".to_string()),
        enabled: true,
        kind: ScheduleKind::Agent,
        target: "act".to_string(),
        params,
        overlap: ScheduleOverlap::Skip,
        node_id: Some("node-a".to_string()),
    }
}

fn srec(schedule: &str, for_ms: i64, status: &str) -> ScheduleRunRecord {
    ScheduleRunRecord {
        schedule_id: schedule.to_string(),
        kind: "agent".into(),
        target: "act".into(),
        scheduled_for_ms: for_ms,
        fired_at_ms: for_ms + 42,
        execution_id: (status == SCHEDULE_RUN_FIRED).then(|| format!("{schedule}-{for_ms}")),
        status: status.to_string(),
        error: (status == SCHEDULE_RUN_ERROR).then(|| "submit refused".into()),
        missed: status == SCHEDULE_RUN_MISSED,
    }
}

fn trec(topic: &str, node_id: &str, status: &str, created_at: i64) -> TeamTopicRunRecord {
    TeamTopicRunRecord {
        topic_id: topic.to_string(),
        node_id: node_id.to_string(),
        status: status.to_string(),
        created_at,
    }
}

// ── schedule definitions ─────────────────────────────────────────────────

#[tokio::test]
async fn schedule_upsert_get_roundtrips_the_job_body() {
    let store = mem().await;
    store.upsert_schedule(&job("nightly"), 1_000).await.unwrap();

    let def = store.get_schedule("nightly").await.unwrap().unwrap();
    assert_eq!(def.job, job("nightly"));
    assert_eq!(def.created_at, 1_000);
    assert_eq!(def.updated_at, 1_000);

    // Every definition is listed; the unknown id misses gracefully.
    assert_eq!(store.list_schedules().await.unwrap().len(), 1);
    assert!(store.get_schedule("ghost").await.unwrap().is_none());
}

#[tokio::test]
async fn schedule_upsert_preserves_created_at_and_bumps_updated_at() {
    let store = mem().await;
    store.upsert_schedule(&job("nightly"), 1_000).await.unwrap();
    let mut next = job("nightly");
    next.cron = "*/5 * * * *".to_string();
    next.overlap = ScheduleOverlap::Allow;
    next.node_id = None;
    store.upsert_schedule(&next, 9_000).await.unwrap();

    let def = store.get_schedule("nightly").await.unwrap().unwrap();
    assert_eq!(def.job.cron, "*/5 * * * *");
    assert_eq!(def.job.overlap, ScheduleOverlap::Allow);
    assert_eq!(def.job.node_id, None);
    assert_eq!(def.created_at, 1_000, "creation metadata is server-owned");
    assert_eq!(def.updated_at, 9_000);
    assert_eq!(
        store.list_schedules().await.unwrap().len(),
        1,
        "upsert, not duplicate"
    );
}

#[tokio::test]
async fn schedule_list_is_stable_id_order() {
    let store = mem().await;
    for id in ["zebra", "alpha", "midway"] {
        store.upsert_schedule(&job(id), 1_000).await.unwrap();
    }
    let ids: Vec<String> = store
        .list_schedules()
        .await
        .unwrap()
        .into_iter()
        .map(|def| def.job.id)
        .collect();
    assert_eq!(ids, vec!["alpha", "midway", "zebra"]);
}

#[tokio::test]
async fn schedule_delete_removes_the_definition_but_keeps_the_fire_ledger() {
    let store = mem().await;
    store.upsert_schedule(&job("nightly"), 1_000).await.unwrap();
    store
        .record_schedule_run(&srec("nightly", 2_000, SCHEDULE_RUN_FIRED))
        .await
        .unwrap();

    store.delete_schedule("nightly").await.unwrap();
    assert!(store.get_schedule("nightly").await.unwrap().is_none());
    assert!(store.list_schedules().await.unwrap().is_empty());
    // No FK: the ledger outlives the definition (audit trail).
    let runs = store.list_schedule_runs("nightly", 10).await.unwrap();
    assert_eq!(runs.len(), 1, "fire history outlives the definition");
}

// ── schedule runs (fire ledger) ──────────────────────────────────────────

#[tokio::test]
async fn schedule_runs_roundtrip_newest_first_with_limit() {
    let store = mem().await;
    for ms in [1_000, 2_000, 3_000] {
        store
            .record_schedule_run(&srec("nightly", ms, SCHEDULE_RUN_FIRED))
            .await
            .unwrap();
    }
    // One missed tick interleaved (older than the fired catch-up tick).
    store
        .record_schedule_run(&srec("nightly", 500, SCHEDULE_RUN_MISSED))
        .await
        .unwrap();

    let all = store.list_schedule_runs("nightly", 100).await.unwrap();
    assert_eq!(all.len(), 4);
    assert_eq!(
        all.iter().map(|r| r.scheduled_for_ms).collect::<Vec<_>>(),
        vec![3_000, 2_000, 1_000, 500],
        "newest tick first"
    );
    assert_eq!(all[0].execution_id.as_deref(), Some("nightly-3000"));
    assert_eq!(all[0].status, SCHEDULE_RUN_FIRED);
    assert_eq!(all[0].fired_at_ms, 3_042);
    assert!(all[3].missed, "skipped tick is flagged");

    let two = store.list_schedule_runs("nightly", 2).await.unwrap();
    assert_eq!(two.len(), 2);
    assert_eq!(two[0].scheduled_for_ms, 3_000);
}

#[tokio::test]
async fn schedule_record_is_idempotent_per_tick() {
    let store = mem().await;
    store
        .record_schedule_run(&srec("daily", 1_000, SCHEDULE_RUN_ERROR))
        .await
        .unwrap();
    // Same tick retried after the error: converges, does not duplicate.
    store
        .record_schedule_run(&srec("daily", 1_000, SCHEDULE_RUN_FIRED))
        .await
        .unwrap();

    let runs = store.list_schedule_runs("daily", 100).await.unwrap();
    assert_eq!(runs.len(), 1, "same tick replaces in place");
    assert_eq!(runs[0].status, SCHEDULE_RUN_FIRED);
    assert!(runs[0].error.is_none());
}

#[tokio::test]
async fn schedule_last_returns_newest_tick_or_none() {
    let store = mem().await;
    assert!(store.last_schedule_run("quiet").await.unwrap().is_none());

    store
        .record_schedule_run(&srec("quiet", 1_000, SCHEDULE_RUN_FIRED))
        .await
        .unwrap();
    store
        .record_schedule_run(&srec("quiet", 2_000, SCHEDULE_RUN_FIRED))
        .await
        .unwrap();
    store
        .record_schedule_run(&srec("other", 5_000, SCHEDULE_RUN_FIRED))
        .await
        .unwrap();

    let last = store.last_schedule_run("quiet").await.unwrap().unwrap();
    assert_eq!(last.scheduled_for_ms, 2_000, "latest tick wins");
    assert_eq!(
        store
            .last_schedule_run("other")
            .await
            .unwrap()
            .unwrap()
            .scheduled_for_ms,
        5_000
    );
    assert!(store.last_schedule_run("ghost").await.unwrap().is_none());
}

#[tokio::test]
async fn schedule_runs_have_no_node_fk() {
    // The team ledger cascades with nodes; the schedule ledger records
    // fires for definitions that may outlive any node, so it must accept
    // rows without any node registry entry at all.
    let store = mem().await;
    store
        .record_schedule_run(&srec("no-nodes", 1_000, SCHEDULE_RUN_FIRED))
        .await
        .unwrap();
    assert!(store.last_schedule_run("no-nodes").await.unwrap().is_some());
}

// ── team topic runs ──────────────────────────────────────────────────────

#[tokio::test]
async fn team_runs_roundtrip_and_created_at_stability() {
    let store = mem().await;
    let n1 = register(&store, "node-alpha", 1_000).await.id;
    let n2 = register(&store, "node-beta", 1_000).await.id;

    // Two rows, same topic, different nodes.
    store
        .upsert_team_topic_run(&trec("topic-1", &n1, TEAM_RUN_EXECUTING, 1_000))
        .await
        .unwrap();
    store
        .upsert_team_topic_run(&trec("topic-1", &n2, TEAM_RUN_EXECUTING, 1_100))
        .await
        .unwrap();

    let runs = store.list_team_topic_runs("topic-1").await.unwrap();
    assert_eq!(runs.len(), 2, "both (topic, node) pairings are listed");
    assert_eq!(runs[0].node_id, n1, "oldest created_at first");
    assert_eq!(runs[0].status, TEAM_RUN_EXECUTING);
    assert_eq!(runs[0].created_at, 1_000);
    assert_eq!(runs[1].node_id, n2);
    assert_eq!(runs[1].created_at, 1_100);

    // Re-upsert an existing row: status moves, created_at is preserved.
    store
        .upsert_team_topic_run(&trec("topic-1", &n1, TEAM_RUN_FINISHED, 9_999))
        .await
        .unwrap();
    let runs = store.list_team_topic_runs("topic-1").await.unwrap();
    assert_eq!(runs.len(), 2, "upsert replaces, never duplicates");
    assert_eq!(
        runs[0].created_at, 1_000,
        "created_at keeps its first-insert value across upserts"
    );
    assert_eq!(runs[0].status, TEAM_RUN_FINISHED, "status is refreshed");
    assert_eq!(runs[1].status, TEAM_RUN_EXECUTING, "sibling row untouched");

    // Other topics are isolated.
    let n3 = register(&store, "node-gamma", 1_000).await.id;
    store
        .upsert_team_topic_run(&trec("topic-2", &n3, TEAM_RUN_EXECUTING, 1_200))
        .await
        .unwrap();
    assert_eq!(
        store.list_team_topic_runs("topic-1").await.unwrap().len(),
        2,
        "topic-2's row must not leak into topic-1"
    );
    assert_eq!(
        store.list_team_topic_runs("topic-2").await.unwrap().len(),
        1
    );
    assert_eq!(
        store
            .list_team_topic_runs("topic-unknown")
            .await
            .unwrap()
            .len(),
        0,
        "unknown topic lists empty"
    );

    // Bulk finish flips every row; unknown topics are a no-op.
    store.finish_team_topic_run("topic-1").await.unwrap();
    let runs = store.list_team_topic_runs("topic-1").await.unwrap();
    assert!(runs.iter().all(|r| r.status == TEAM_RUN_FINISHED));
    store.finish_team_topic_run("topic-none").await.unwrap();
}

#[tokio::test]
async fn team_runs_reject_unknown_nodes_fk_parity() {
    let store = mem().await;
    assert!(
        store
            .upsert_team_topic_run(&trec("topic-x", "ghost-node", TEAM_RUN_EXECUTING, 1))
            .await
            .is_err(),
        "fk parity: rows pair with registered nodes only"
    );
    assert!(store
        .list_team_topic_runs("topic-x")
        .await
        .unwrap()
        .is_empty());
}

/// The node FK cascade: deleting a node removes its pairings (realized as
/// GC-on-list here) while sibling pairings survive.
#[tokio::test]
async fn team_runs_node_deletion_cascades_to_its_topic_runs() {
    let store = mem().await;
    let n1 = register(&store, "node-alpha", 1_000).await.id;
    let n2 = register(&store, "node-beta", 1_000).await.id;
    store
        .upsert_team_topic_run(&trec("topic-c", &n1, TEAM_RUN_EXECUTING, 1_000))
        .await
        .unwrap();
    store
        .upsert_team_topic_run(&trec("topic-c", &n2, TEAM_RUN_EXECUTING, 1_100))
        .await
        .unwrap();

    store.delete_node(&n1).await.unwrap();
    let runs = store.list_team_topic_runs("topic-c").await.unwrap();
    assert_eq!(
        runs.len(),
        1,
        "deleted node's pairing cascades away, sibling survives"
    );
    assert_eq!(runs[0].node_id, n2);
}
