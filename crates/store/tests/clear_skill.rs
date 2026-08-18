//! `SessionPatch::clear_skill` lets a caller NULL out the `skill` field.
//! After a skill-scoped run completes the active skill must be clearable,
//! which the plain `Option<T>` (None = "skip") semantics cannot express —
//! hence the dedicated `clear_skill: true` flag is needed.

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
        skill: Some("reviewer".into()),
        task_type: None,
        requirement: None,
        plan_snapshot: None,
        plan_input_count: 0,
    }
}

#[tokio::test]
async fn clear_skill_nulls_skill_field() {
    let store = mem().await;
    store.create_session(&meta("s1")).await.unwrap();

    // Sanity: skill is populated.
    let before = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(before.skill.as_deref(), Some("reviewer"));

    // Clear it via the dedicated flag (skill: None still means "don't touch").
    let patch = SessionPatch {
        clear_skill: true,
        ..Default::default()
    };
    store.update_session("s1", &patch).await.unwrap();

    let after = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(after.skill, None, "skill must be cleared");
    // Unrelated field is untouched.
    assert_eq!(after.title.as_deref(), Some("s1"));
}

#[tokio::test]
async fn default_patch_leaves_skill_intact() {
    let store = mem().await;
    store.create_session(&meta("s2")).await.unwrap();

    // A patch that sets a title but does NOT request a skill clear.
    let patch = SessionPatch {
        title: Some("renamed".into()),
        ..Default::default()
    };
    store.update_session("s2", &patch).await.unwrap();

    let after = store.get_session("s2").await.unwrap().unwrap();
    assert_eq!(after.title.as_deref(), Some("renamed"));
    // skill survives unchanged.
    assert_eq!(after.skill.as_deref(), Some("reviewer"));
}
