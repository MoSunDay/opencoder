//! Terminal-path flows: cooperative cancellation mid-run and capability
//! profiling fan-out. Both finish work through the same durable paths the
//! happy-path tests assert on (team.json on the share, `team_topic_runs`).

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use common::*;
use opencoder_store::{Store, TeamTopicRunRecord, TEAM_RUN_EXECUTING, TEAM_RUN_FINISHED};
use opencoder_team::{
    err, fs_store, ok, profile_team, CancelToken, MockDispatcher, TeamDispatcher, TeamMember,
};
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

/// Scripts one JSON profile reply per node; on the first `ask`, before
/// replying, simulates a concurrent management edit: a third registered
/// member is appended to `team.json` on the share (interior mutability, no
/// classes/inheritance).
struct ConcurrentAddDispatcher {
    root: PathBuf,
    third_id: String,
    third_name: String,
    replies: Mutex<HashMap<String, String>>,
    edited: AtomicBool,
}

#[async_trait]
impl TeamDispatcher for ConcurrentAddDispatcher {
    async fn ask(&self, _topic: Option<&str>, node_id: &str, _prompt: &str) -> Result<String> {
        if !self.edited.swap(true, Ordering::SeqCst) {
            let mut team = fs_store::load_team(&self.root, TEAM).unwrap();
            team.members.push(TeamMember {
                node_id: self.third_id.clone(),
                name: self.third_name.clone(),
                capabilities: Vec::new(),
                profiled_at: None,
            });
            team.updated_at = 2_000;
            fs_store::save_team(&self.root, &team).unwrap();
        }
        self.replies
            .lock()
            .expect("scripted replies lock")
            .remove(node_id)
            .ok_or_else(|| anyhow!("no scripted reply left for node {node_id}"))
    }
}

#[tokio::test]
async fn profile_narrow_merge_preserves_concurrent_membership_edit() {
    let fx = fixture(3, 2).await;
    let (captain, members) = make_team(&fx, 2).await;
    let third = register(&fx.store, "late-member").await;

    let mut replies = HashMap::new();
    replies.insert(
        members[0].id.clone(),
        json!({"capabilities": ["Rust 异步运行时"]}).to_string(),
    );
    replies.insert(
        members[1].id.clone(),
        json!({"capabilities": ["前端 React"]}).to_string(),
    );
    let dispatcher = Arc::new(ConcurrentAddDispatcher {
        root: fx.root().to_path_buf(),
        third_id: third.id.clone(),
        third_name: third.name.clone(),
        replies: Mutex::new(replies),
        edited: AtomicBool::new(false),
    });

    let meta = profile_team(dispatcher, &fx.cfg, TEAM).await.unwrap();

    assert_eq!(meta.members.len(), 3, "concurrent membership add survives");
    let member_of = |id: &str| meta.members.iter().find(|m| m.node_id == id).unwrap();
    let first = member_of(&members[0].id);
    assert_eq!(first.capabilities, vec!["Rust 异步运行时"]);
    assert!(first.profiled_at.is_some());
    let second = member_of(&members[1].id);
    assert_eq!(second.capabilities, vec!["前端 React"]);
    assert!(second.profiled_at.is_some());
    let added = member_of(&third.id);
    assert!(
        added.capabilities.is_empty(),
        "concurrently added member untouched by profiling"
    );
    assert!(added.profiled_at.is_none());
    assert_eq!(meta.captain.node_id, captain.id, "captain unchanged");
}
