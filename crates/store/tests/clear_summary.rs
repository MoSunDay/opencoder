//! `SessionPatch::clear_summary` lets a caller NULL out the compaction
//! metadata (`summary`, `summary_seq`, `summary_images_json`) in a single
//! update -- the symmetric counterpart to `clear_handoff`. Plan->act handoff
//! supersedes an earlier compaction, so the handoff persistence path sets
//! `clear_summary: true`; without this flag the residual `summary_seq` would
//! corrupt the next compaction's OFFSET (`prev_skip = summary_seq.or(handoff_seq)`).
//!
//! Also covers the `create()` INSERT now binding `summary_images_json` (Fix 3):
//! before that, a freshly-created session lost its image list on read-back.

use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};

async fn mem() -> LibsqlStore {
    LibsqlStore::open_memory().await.unwrap()
}

/// A session carrying BOTH compaction and handoff metadata -- the dirty state
/// that exists between a compaction and the subsequent handoff that supersedes it.
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
        summary: Some("compacted history".into()),
        summary_seq: Some(10),
        summary_images: vec!["img1.png".into(), "img2.png".into()],
        handoff_seq: Some(42),
        handoff_plan: Some("do-the-thing".into()),
        skill: None,
        task_type: None,
        requirement: None,
    }
}

#[tokio::test]
async fn clear_summary_nulls_all_compaction_fields() {
    let store = mem().await;
    store.create_session(&meta("s1")).await.unwrap();

    // Sanity: all three compaction fields are populated.
    let before = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(before.summary_seq, Some(10));
    assert_eq!(before.summary.as_deref(), Some("compacted history"));
    assert_eq!(
        before.summary_images,
        vec!["img1.png".to_string(), "img2.png".to_string()]
    );

    // Clear them via the dedicated flag (None still means "don't touch").
    let patch = SessionPatch {
        clear_summary: true,
        ..Default::default()
    };
    store.update_session("s1", &patch).await.unwrap();

    let after = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(after.summary_seq, None, "summary_seq must be cleared");
    assert_eq!(after.summary, None, "summary text must be cleared");
    assert!(
        after.summary_images.is_empty(),
        "summary_images must be cleared"
    );
    // An unrelated field is untouched.
    assert_eq!(after.title.as_deref(), Some("s1"));
}

/// Mirrors the real `persist_clear` / autopilot-handoff call shape: a single
/// update that sets the NEW handoff boundary AND clears the STALE compaction
/// metadata. Both must coexist in one patch without clobbering each other.
#[tokio::test]
async fn clear_summary_coexists_with_handoff_update() {
    let store = mem().await;
    store.create_session(&meta("s2")).await.unwrap();

    // Exactly the shape control_cmd::persist_clear emits: new handoff fields
    // plus clear_summary to drop the superseded compaction.
    let patch = SessionPatch {
        handoff_seq: Some(28),
        handoff_plan: Some("## Plan\n1. execute".into()),
        clear_summary: true,
        updated_at: Some(1),
        ..Default::default()
    };
    store.update_session("s2", &patch).await.unwrap();

    let after = store.get_session("s2").await.unwrap().unwrap();
    // Handoff got the new value.
    assert_eq!(
        after.handoff_seq,
        Some(28),
        "handoff_seq takes the new value"
    );
    assert_eq!(after.handoff_plan.as_deref(), Some("## Plan\n1. execute"));
    // Compaction metadata is gone despite also being present before the update.
    assert_eq!(
        after.summary_seq, None,
        "stale summary_seq cleared in the same patch"
    );
    assert_eq!(
        after.summary, None,
        "stale summary cleared in the same patch"
    );
    assert!(
        after.summary_images.is_empty(),
        "stale summary_images cleared in the same patch"
    );
}

/// Fix 3: `create()` now binds `summary_images_json`. A session created with a
/// non-empty image list must round-trip through `get_session` -- before the fix
/// the INSERT omitted the column and the list was silently lost on read-back.
#[tokio::test]
async fn create_persists_summary_images() {
    let store = mem().await;
    store
        .create_session(&SessionMeta {
            id: "s3".into(),
            title: Some("s3".into()),
            agent: Some("act".into()),
            model: Some("m".into()),
            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec!["create-a.png".into(), "create-b.png".into()],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();

    // No intervening update_session -- this exercises the INSERT path only.
    let got = store.get_session("s3").await.unwrap().unwrap();
    assert_eq!(
        got.summary_images,
        vec!["create-a.png".to_string(), "create-b.png".to_string()],
        "summary_images must survive create->get round-trip (INSERT binds the column)",
    );
}
