//! Plan-phase persistence round-trips: `plan_snapshot` (compaction-captured
//! plan text) and `plan_input_count` (plan-phase arming counter) must survive
//! a `SessionPatch` write + `get_session` read cycle, including the explicit
//! clear path, and the snapshot value/clear flags must stay mutually
//! exclusive (validated like every other column pair).

use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};

async fn store_with_session() -> (tempfile::TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("t.db")).await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "s1".into(),
            title: Some("t".into()),
            agent: Some("plan".into()),
            model: Some("m".into()),
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
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .unwrap();
    (dir, store)
}

#[tokio::test]
async fn plan_snapshot_round_trip_via_patch() {
    let (_dir, store) = store_with_session().await;
    store
        .update_session(
            "s1",
            &SessionPatch {
                plan_snapshot: Some("## Plan\n1. step".into()),
                plan_input_count: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let meta = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(meta.plan_snapshot.as_deref(), Some("## Plan\n1. step"));
    assert_eq!(meta.plan_input_count, 2);

    // Clear the snapshot without touching the counter.
    store
        .update_session(
            "s1",
            &SessionPatch {
                clear_plan_snapshot: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let meta = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(meta.plan_snapshot, None, "clear_plan_snapshot must NULL it");
    assert_eq!(meta.plan_input_count, 2, "counter untouched by the clear");

    // Counter can go back to zero (post-handoff mirror).
    store
        .update_session(
            "s1",
            &SessionPatch {
                plan_input_count: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let meta = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(meta.plan_input_count, 0);
}

#[tokio::test]
async fn plan_snapshot_set_and_clear_are_mutually_exclusive() {
    let (_dir, store) = store_with_session().await;
    let err = store
        .update_session(
            "s1",
            &SessionPatch {
                plan_snapshot: Some("x".into()),
                clear_plan_snapshot: true,
                ..Default::default()
            },
        )
        .await;
    assert!(err.is_err(), "value + clear must be rejected");
}

#[tokio::test]
async fn create_session_carries_plan_phase_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("t.db")).await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "fresh".into(),
            plan_snapshot: Some("carried plan".into()),
            plan_input_count: 4,
            ..SessionMeta::default()
        })
        .await
        .unwrap();
    let meta = store.get_session("fresh").await.unwrap().unwrap();
    assert_eq!(meta.plan_snapshot.as_deref(), Some("carried plan"));
    assert_eq!(meta.plan_input_count, 4);
}
