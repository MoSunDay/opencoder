//! Bug #15: a `SessionPatch` that simultaneously sets a column value (`Some(..)`)
//! and requests the same column be cleared (`clear_*: true`) would otherwise
//! emit contradictory SET clauses — e.g. both `summary = ?` and `summary = NULL`
//! — whose final effect depends on clause ordering in the generated SQL. The
//! `update` path now rejects these mutually-exclusive combinations up front.
//!
//! The `clear_summary` flag NULLs `summary`, `summary_seq`, AND
//! `summary_images_json`; `clear_handoff` NULLs `handoff_seq` AND `handoff_plan`.
//! Setting any of those fields together with the matching clear flag is rejected.

use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};

async fn mem() -> LibsqlStore {
    let store = LibsqlStore::open_memory().await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "s1".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    store
}

/// Each conflicting pair must produce an error. The session still exists in
/// every case; only the patch is contradictory, so `Err` is the expected result.
#[tokio::test]
async fn field_and_clear_combinations_are_rejected() {
    let cases: Vec<(&str, SessionPatch)> = vec![
        (
            "summary + clear_summary",
            SessionPatch {
                summary: Some("s".into()),
                clear_summary: true,
                ..Default::default()
            },
        ),
        (
            "summary_seq + clear_summary",
            SessionPatch {
                summary_seq: Some(5),
                clear_summary: true,
                ..Default::default()
            },
        ),
        (
            "summary_images + clear_summary",
            SessionPatch {
                summary_images: Some(vec!["i.png".into()]),
                clear_summary: true,
                ..Default::default()
            },
        ),
        (
            "handoff_plan + clear_handoff",
            SessionPatch {
                handoff_plan: Some("plan".into()),
                clear_handoff: true,
                ..Default::default()
            },
        ),
        (
            "handoff_seq + clear_handoff",
            SessionPatch {
                handoff_seq: Some(7),
                clear_handoff: true,
                ..Default::default()
            },
        ),
        (
            "skill + clear_skill",
            SessionPatch {
                skill: Some("reviewer".into()),
                clear_skill: true,
                ..Default::default()
            },
        ),
    ];

    for (name, patch) in cases {
        let store = mem().await;
        let result = store.update_session("s1", &patch).await;
        assert!(result.is_err(), "{name}: conflicting field+clear must error");
    }
}

/// A non-conflicting patch (clear a field while setting an unrelated one) must
/// still succeed — the guard only rejects same-column contradictions.
#[tokio::test]
async fn unrelated_field_and_clear_still_succeeds() {
    let store = mem().await;
    // Set a title while clearing the handoff — different columns, no conflict.
    let patch = SessionPatch {
        title: Some("renamed".into()),
        clear_handoff: true,
        ..Default::default()
    };
    store.update_session("s1", &patch).await.unwrap();
    let after = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(after.title.as_deref(), Some("renamed"));
}

/// A patch that sets a clear flag without the corresponding field, plus no
/// other contradictory input, must succeed.
#[tokio::test]
async fn clear_flag_alone_succeeds() {
    let store = mem().await;
    let patch = SessionPatch {
        clear_skill: true,
        ..Default::default()
    };
    store.update_session("s1", &patch).await.unwrap();
}
