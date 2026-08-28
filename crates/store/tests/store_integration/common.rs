//! Shared helpers for the `store_integration` test suite.

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use tempfile::TempDir;

pub(crate) fn conv(seed: &str, n: usize) -> Vec<Message> {
    (0..n)
        .map(|i| {
            let id = format!("{seed}-{i}");
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            let text = format!("{seed} msg {i}");
            let mut m = match role {
                Role::User => Message::user(id, text),
                Role::Assistant => {
                    let mut m = Message::assistant(id);
                    m.blocks = vec![ContentBlock::text(text)];
                    m
                }
                _ => unreachable!(),
            };
            m.created_at = i as i64;
            m
        })
        .collect()
}

pub(crate) async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

pub(crate) async fn make_session(store: &LibsqlStore, id: &str, now: i64) {
    let meta = SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("glm-5.2".into()),
        autopilot_mode: None,
        workdir_hash: Some("h".into()),
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    };
    store.create_session(&meta).await.unwrap();
}
