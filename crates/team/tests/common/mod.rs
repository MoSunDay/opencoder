#![allow(dead_code)] // shared across test binaries; each sees only its own subset
//! Shared fixtures: tempdir team root ("fake NFS"), real LibsqlStore, a
//! registered-node team, and topic bootstrapping helpers.

use std::sync::Arc;
use std::time::Duration;

use opencoder_store::{LibsqlStore, NodeRecord, Store};
use opencoder_team::{
    fs_store,
    types::{MemberRef, TeamMember, TeamMeta, TopicMeta},
    CancelToken, MockDispatcher, NodeDispatcher, TeamRunConfig,
};
use tempfile::TempDir;

pub const TEAM: &str = "demo";

pub struct Fixture {
    pub root: TempDir,
    pub db: TempDir,
    pub store: Arc<LibsqlStore>,
    pub cfg: TeamRunConfig,
}

impl Fixture {
    pub fn root(&self) -> &std::path::Path {
        self.root.path()
    }
}

pub async fn fixture(max_turns: usize, max_sub_turns: usize) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap();
    let store = Arc::new(LibsqlStore::open(db.path().join("team.db")).await.unwrap());
    let cfg = TeamRunConfig {
        team_root: root.path().to_path_buf(),
        max_turns,
        max_sub_turns,
    };
    Fixture {
        root,
        db,
        store,
        cfg,
    }
}

pub async fn register(store: &LibsqlStore, name: &str) -> NodeRecord {
    store
        .register_node(name, Some("v1"), Some("/tmp/wd"), None, 1_000)
        .await
        .unwrap()
}

/// Create the demo team: captain + `member_count` members, all registered
/// nodes (ids are server-issued ULIDs, exactly like production).
pub async fn make_team(fx: &Fixture, member_count: usize) -> (NodeRecord, Vec<NodeRecord>) {
    let captain = register(&fx.store, "captain").await;
    let mut members = Vec::new();
    for i in 0..member_count {
        members.push(register(&fx.store, &format!("member-{i}")).await);
    }
    let meta = team_meta(&captain, &members);
    fs_store::create_team(fx.root(), &meta).unwrap();
    (captain, members)
}

pub fn team_meta(captain: &NodeRecord, members: &[NodeRecord]) -> TeamMeta {
    TeamMeta {
        name: TEAM.to_string(),
        captain: MemberRef {
            node_id: captain.id.clone(),
            name: captain.name.clone(),
        },
        members: members
            .iter()
            .map(|n| TeamMember {
                node_id: n.id.clone(),
                name: n.name.clone(),
                capabilities: Vec::new(),
                profiled_at: None,
            })
            .collect(),
        created_at: 1_000,
        updated_at: 1_000,
    }
}

/// Start a topic on the demo team and return its ULID.
pub async fn start(fx: &Fixture, title: &str) -> String {
    let meta =
        opencoder_team::start_topic(fx.store.clone(), &fx.cfg, TEAM, title, "调研并给出方案")
            .await
            .unwrap();
    meta.topic_id
}

/// run_topic with the given scripted dispatcher (kept behind an `Arc` so the
/// test can still inspect the recorded calls afterwards).
pub async fn run(fx: &Fixture, mock: Arc<MockDispatcher>, topic_id: &str) -> TopicMeta {
    opencoder_team::run_topic(
        fx.store.clone(),
        mock,
        &fx.cfg,
        TEAM,
        topic_id,
        CancelToken::new(),
    )
    .await
    .unwrap()
}

pub async fn run_cancelled(
    fx: &Fixture,
    mock: Arc<MockDispatcher>,
    topic_id: &str,
    token: CancelToken,
) -> TopicMeta {
    opencoder_team::run_topic(fx.store.clone(), mock, &fx.cfg, TEAM, topic_id, token)
        .await
        .unwrap()
}

/// A real NodeDispatcher with test-friendly polling (for dispatcher tests).
pub fn fast_dispatcher(store: Arc<LibsqlStore>) -> NodeDispatcher {
    NodeDispatcher::with_timeouts(store, Duration::from_millis(20), Duration::from_secs(5))
}
