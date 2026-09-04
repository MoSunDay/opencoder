//! Terminal-path flows: cooperative cancellation mid-run and capability
//! profiling fan-out. Both finish work through the same durable paths the
//! happy-path tests assert on (team.json on the share, `team_topic_runs`).

mod common;

use std::sync::Arc;

use common::*;
use opencoder_store::{Store, TeamTopicRunRecord, TEAM_RUN_EXECUTING, TEAM_RUN_FINISHED};
use opencoder_team::{err, fs_store, ok, profile_team, CancelToken, MockDispatcher};
use serde_json::json;

#[tokio::test]
async fn cancellation_finishes_topic_and_ledger() {
    let fx = fixture(3, 2).await;
    let (_captain, members) = make_team(&fx, 1).await;
    let topic_id = start(&fx, "取消").await;

    // An in-flight ledger row exists (as if a member were mid-answer).
    fx.store
        .upsert_team_topic_run(&TeamTopicRunRecord {
            topic_id: topic_id.clone(),
            node_id: members[0].id.clone(),
            status: TEAM_RUN_EXECUTING.to_string(),
            created_at: 1,
        })
        .await
        .unwrap();

    let token = CancelToken::new();
    token.cancel();
    let meta = run_cancelled(&fx, Arc::new(MockDispatcher::new()), &topic_id, token).await;
    assert_eq!(meta.status, "finished");
    assert_eq!(meta.finish_reason.as_deref(), Some("cancelled"));
    assert!(meta.finished_at.is_some());
    let rows = fx.store.list_team_topic_runs(&topic_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, TEAM_RUN_FINISHED);
}

#[tokio::test]
async fn profile_team_fans_out_and_persists_capabilities() {
    let fx = fixture(3, 2).await;
    let (_captain, members) = make_team(&fx, 3).await;

    let mock = Arc::new(
        MockDispatcher::new()
            .reply(
                &members[0].id,
                vec![
                    ok("我擅长很多东西"),
                    ok(json!({"capabilities": ["Rust 异步运行时", "libsql 迁移"]}).to_string()),
                ],
            )
            .reply(
                &members[1].id,
                vec![ok(json!({"capabilities": ["前端 React"]}).to_string())],
            )
            .reply(&members[2].id, vec![err("画像失败")]),
    );
    let meta = profile_team(mock.clone(), &fx.cfg, TEAM).await.unwrap();

    assert_eq!(
        meta.members[0].capabilities,
        vec!["Rust 异步运行时", "libsql 迁移"]
    );
    assert!(meta.members[0].profiled_at.is_some());
    assert_eq!(meta.members[1].capabilities, vec!["前端 React"]);
    assert!(
        meta.members[2].capabilities.is_empty(),
        "failed member keeps empty capabilities"
    );
    assert!(meta.members[2].profiled_at.is_none());

    assert_eq!(
        mock.calls_for(&members[0].id).len(),
        2,
        "garbage first reply -> correction re-ask"
    );
    assert_eq!(mock.calls_for(&members[1].id).len(), 1);
    assert!(
        mock.calls().iter().all(|c| c.topic.is_none()),
        "profiling writes no ledger rows"
    );
    let rows = fx.store.list_team_topic_runs(&fresh_ulid()).await.unwrap();
    assert!(rows.is_empty());

    // Persisted to team.json on the share.
    let disk = fs_store::load_team(fx.root(), TEAM).unwrap();
    assert_eq!(disk.members[0].capabilities.len(), 2);
    assert!(disk.updated_at >= 1_000);
}

fn fresh_ulid() -> String {
    ulid::Ulid::new().to_string()
}
