//! Session lifecycle contracts: CRUD, patch round-trips, keep-one cleanup
//! and handoff/skill field persistence.

use crate::common::{conv, fresh, make_session};
use opencoder_store::{LibsqlStore, SessionFilter, SessionMeta, SessionPatch, Store};

#[tokio::test]
async fn create_get_update_delete_session_contract() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s1", 1000).await;

    let got = store
        .get_session("s1")
        .await
        .unwrap()
        .expect("session exists");
    assert_eq!(got.id, "s1");
    assert_eq!(got.title.as_deref(), Some("title-s1"));
    assert_eq!(got.model.as_deref(), Some("glm-5.2"));

    let patch = opencoder_store::SessionPatch {
        title: Some("renamed".into()),
        model: Some("other/model".into()),
        updated_at: Some(2000),
        ..Default::default()
    };
    store.update_session("s1", &patch).await.unwrap();
    let got = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(got.title.as_deref(), Some("renamed"));
    assert_eq!(got.model.as_deref(), Some("other/model"));
    assert_eq!(got.updated_at, 2000);

    store.delete_session("s1").await.unwrap();
    assert!(store.get_session("s1").await.unwrap().is_none());
}

/// v11: the session-scoped `autopilot_mode` column must survive the full
/// create -> patch -> clear lifecycle, mirroring how `model` is treated.
#[tokio::test]
async fn autopilot_mode_column_round_trips() {
    let (_dir, store) = fresh().await;
    let meta = SessionMeta {
        id: "s-ap".into(),
        autopilot_mode: Some("ap".into()),
        ..Default::default()
    };
    store.create_session(&meta).await.unwrap();
    assert_eq!(
        store
            .get_session("s-ap")
            .await
            .unwrap()
            .unwrap()
            .autopilot_mode
            .as_deref(),
        Some("ap"),
        "created autopilot_mode must round-trip"
    );

    store
        .update_session(
            "s-ap",
            &opencoder_store::SessionPatch {
                autopilot_mode: Some("review".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .get_session("s-ap")
            .await
            .unwrap()
            .unwrap()
            .autopilot_mode
            .as_deref(),
        Some("review"),
        "patched autopilot_mode must round-trip"
    );

    store
        .update_session(
            "s-ap",
            &opencoder_store::SessionPatch {
                clear_autopilot_mode: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .get_session("s-ap")
            .await
            .unwrap()
            .unwrap()
            .autopilot_mode,
        None,
        "clear_autopilot_mode must NULL the column"
    );
}

#[tokio::test]
async fn clear_other_sessions_keeps_current_and_cascades() {
    let (_dir, store) = fresh().await;
    make_session(&store, "keep", 1000).await;
    make_session(&store, "old-a", 2000).await;
    make_session(&store, "old-b", 3000).await;
    store
        .append_messages("old-a", &conv("old-a", 2))
        .await
        .unwrap();
    store
        .append_messages("old-b", &conv("old-b", 3))
        .await
        .unwrap();

    let deleted = store.clear_other_sessions("keep").await.unwrap();
    assert_eq!(deleted, 2, "two non-current sessions should be deleted");

    let remaining: Vec<String> = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(remaining, vec!["keep".to_string()]);

    // FK ON DELETE CASCADE removed the child message rows too.
    assert!(
        store.load_messages("old-a").await.unwrap().is_empty(),
        "old-a messages must cascade-delete"
    );
    assert!(
        store.load_messages("old-b").await.unwrap().is_empty(),
        "old-b messages must cascade-delete"
    );
    assert_eq!(
        store.load_messages("keep").await.unwrap().len(),
        0,
        "keep session survives (just had no messages)"
    );

    // Clearing again is a no-op: count 0, keep still present.
    let again = store.clear_other_sessions("keep").await.unwrap();
    assert_eq!(again, 0);
    assert_eq!(
        store
            .list_sessions(&SessionFilter::default())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn session_handoff_and_skill_fields_round_trip() {
    let store = LibsqlStore::open_memory().await.unwrap();
    let id = "rt-session";
    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();

    // Initially null.
    let m0 = store.get_session(id).await.unwrap().unwrap();
    assert!(m0.handoff_seq.is_none());
    assert!(m0.handoff_plan.is_none());
    assert!(m0.skill.is_none());

    // Persist via SessionPatch.
    store
        .update_session(
            id,
            &SessionPatch {
                handoff_seq: Some(7),
                handoff_plan: Some("## Plan\n1. x".into()),
                skill: Some("be terse".into()),
                updated_at: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let m1 = store.get_session(id).await.unwrap().unwrap();
    assert_eq!(m1.handoff_seq, Some(7));
    assert_eq!(m1.handoff_plan.as_deref(), Some("## Plan\n1. x"));
    assert_eq!(m1.skill.as_deref(), Some("be terse"));
    // Untouched fields preserved.
    assert_eq!(m1.agent.as_deref(), Some("act"));
    assert!(m1.summary_seq.is_none());
}
