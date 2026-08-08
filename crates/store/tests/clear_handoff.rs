//! `SessionPatch::clear_handoff` lets a caller NULL out `handoff_seq` and
//! `handoff_plan`. After plan→act handoff compaction these fields must be
//! clearable, which the plain `Option<T>` (None = "skip") semantics cannot
//! express.

use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};

async fn mem() -> LibsqlStore {
    LibsqlStore::open_memory().await.unwrap()
}

fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some(id.into()),
        agent: Some("act".into()),
        model: Some("m".into()),
        workdir_hash: None,
        created_at: 0,
        updated_at: 0,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: Some(42),
        handoff_plan: Some("do-the-thing".into()),
        skill: None,
        task_type: None,
        requirement: None,
    }
}

#[tokio::test]
async fn clear_handoff_nulls_handoff_fields() {
    let store = mem().await;
    store.create_session(&meta("s1")).await.unwrap();

    // Sanity: fields are populated.
    let before = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(before.handoff_seq, Some(42));
    assert_eq!(before.handoff_plan.as_deref(), Some("do-the-thing"));

    // Clear them via the dedicated flag (None still means "don't touch").
    let patch = SessionPatch {
        clear_handoff: true,
        ..Default::default()
    };
    store.update_session("s1", &patch).await.unwrap();

    let after = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(after.handoff_seq, None, "handoff_seq must be cleared");
    assert_eq!(after.handoff_plan, None, "handoff_plan must be cleared");
    // Unrelated field is untouched.
    assert_eq!(after.title.as_deref(), Some("s1"));
}

#[tokio::test]
async fn default_patch_leaves_handoff_fields_intact() {
    let store = mem().await;
    store.create_session(&meta("s2")).await.unwrap();

    // A patch that sets a title but does NOT request a handoff clear.
    let patch = SessionPatch {
        title: Some("renamed".into()),
        ..Default::default()
    };
    store.update_session("s2", &patch).await.unwrap();

    let after = store.get_session("s2").await.unwrap().unwrap();
    assert_eq!(after.title.as_deref(), Some("renamed"));
    // handoff fields survive unchanged.
    assert_eq!(after.handoff_seq, Some(42));
    assert_eq!(after.handoff_plan.as_deref(), Some("do-the-thing"));
}
