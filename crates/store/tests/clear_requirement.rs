//! `SessionPatch::clear_requirement` lets a caller NULL out the `requirement`
//! field (the user annotation). An `/annotation` save with an empty value must
//! explicitly CLEAR the persisted column rather than store an empty string —
//! which the plain `Option<T>` (None = "skip") semantics cannot express, hence
//! the dedicated `clear_requirement: true` flag.

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
        autopilot_mode: None,
        workdir_hash: None,
        created_at: 0,
        updated_at: 0,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: Some(42),
        handoff_plan: Some("do-the-thing".into()),
        skill: Some("reviewer".into()),
        task_type: None,
        requirement: Some("must not leak secrets".into()),
    }
}

#[tokio::test]
async fn clear_requirement_nulls_requirement_field() {
    let store = mem().await;
    store.create_session(&meta("s1")).await.unwrap();

    // Sanity: requirement is populated.
    let before = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(before.requirement.as_deref(), Some("must not leak secrets"));

    // Clear it via the dedicated flag (requirement: None still means "don't touch").
    let patch = SessionPatch {
        clear_requirement: true,
        ..Default::default()
    };
    store.update_session("s1", &patch).await.unwrap();

    let after = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(after.requirement, None, "requirement must be cleared");
    // Unrelated field is untouched.
    assert_eq!(after.title.as_deref(), Some("s1"));
}

#[tokio::test]
async fn default_patch_leaves_requirement_intact() {
    let store = mem().await;
    store.create_session(&meta("s2")).await.unwrap();

    // A patch that sets a title but does NOT request a requirement clear.
    let patch = SessionPatch {
        title: Some("renamed".into()),
        ..Default::default()
    };
    store.update_session("s2", &patch).await.unwrap();

    let after = store.get_session("s2").await.unwrap().unwrap();
    assert_eq!(after.title.as_deref(), Some("renamed"));
    // requirement survives unchanged.
    assert_eq!(after.requirement.as_deref(), Some("must not leak secrets"));
}

#[tokio::test]
async fn clear_then_set_roundtrip() {
    let store = mem().await;
    store.create_session(&meta("s3")).await.unwrap();

    // Clear first...
    let clear = SessionPatch {
        clear_requirement: true,
        ..Default::default()
    };
    store.update_session("s3", &clear).await.unwrap();
    let cleared = store.get_session("s3").await.unwrap().unwrap();
    assert_eq!(cleared.requirement, None);

    // ...then a later patch sets a new value: the clear must not permanently
    // poison the column (a plain value patch still persists).
    let set = SessionPatch {
        requirement: Some("new".into()),
        ..Default::default()
    };
    store.update_session("s3", &set).await.unwrap();

    let after = store.get_session("s3").await.unwrap().unwrap();
    assert_eq!(after.requirement.as_deref(), Some("new"));
}
