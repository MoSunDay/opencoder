//! Active skill survives resume (Gap B).
//!
//! A skill persisted on the session row (via `update_session` with the `skill`
//! patch field) must be restored into `SessionState::skill_prompt` so a resumed
//! session keeps its skill directive instead of silently dropping it.

use std::sync::Arc;

use opencoder_core::Config;
use opencoder_llm::MockChatClient;
use opencoder_session::resume;
use opencoder_store::{LibsqlStore, SessionPatch, Store};

fn cfg() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

async fn seed(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&opencoder_store::SessionMeta {
            id: id.into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
        })
        .await
        .unwrap();
    store
        .append_message(id, &opencoder_core::Message::user("u1", "hi"))
        .await
        .unwrap();
}

#[tokio::test]
async fn resume_restores_persisted_skill() {
    let store = mem_store().await;
    seed(&store, "sk").await;

    // Persist a skill (mirrors TUI/worker SetSkill).
    store
        .update_session(
            "sk",
            &SessionPatch {
                skill: Some("Always answer in haiku form.".into()),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let resumed = resume(
        store,
        "sk",
        cfg(),
        Arc::new(MockChatClient::new()),
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();

    assert_eq!(
        resumed.skill_prompt_cloned().as_deref(),
        Some("Always answer in haiku form."),
        "resume must restore the persisted active skill"
    );
}

#[tokio::test]
async fn resume_without_skill_has_none() {
    let store = mem_store().await;
    seed(&store, "sk2").await;

    let dir = tempfile::tempdir().unwrap();
    let resumed = resume(
        store,
        "sk2",
        cfg(),
        Arc::new(MockChatClient::new()),
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();

    assert!(
        resumed.skill_prompt_cloned().is_none(),
        "no persisted skill -> None on resume"
    );
}
