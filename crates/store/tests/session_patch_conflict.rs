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
        (
            "model + clear_model",
            SessionPatch {
                model: Some("m".into()),
                clear_model: true,
                ..Default::default()
            },
        ),
        (
            "requirement + clear_requirement",
            SessionPatch {
                requirement: Some("r".into()),
                clear_requirement: true,
                ..Default::default()
            },
        ),
        (
            "agent + clear_agent",
            SessionPatch {
                agent: Some("act".into()),
                clear_agent: true,
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

/// `clear_model` / `clear_agent` alone must persist NULL — the whole point of
/// the flags (`model: None` / `agent: None` mean "don't touch", so they can
/// never write NULL). Regression for the web layer's TOCTOU rollback, which
/// relied on a plain `None` field and was a silent no-op for NULL old values.
#[tokio::test]
async fn clear_model_and_clear_agent_null_the_columns() {
    let store = mem().await;
    // Populate both columns first so the clear is observable.
    store
        .update_session(
            "s1",
            &SessionPatch {
                agent: Some("act".into()),
                model: Some("m".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    store
        .update_session(
            "s1",
            &SessionPatch {
                clear_agent: true,
                clear_model: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let after = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(after.agent, None, "clear_agent must NULL the agent column");
    assert_eq!(after.model, None, "clear_model must NULL the model column");
    // Unrelated columns survive.
    assert_eq!(after.id, "s1");
}

/// The rollback constructors must set exactly one of (value, clear) so they
/// both pass the mutual-exclusion guard and always act — restoring the
/// captured value, or clearing when it was NULL / the row was unreadable.
#[test]
fn rollback_constructors_restore_value_or_clear() {
    let old = SessionMeta {
        id: "s".into(),
        agent: Some("act".into()),
        model: Some("m".into()),
        ..Default::default()
    };

    // Captured row with values: restore them, no clear flags.
    let p = SessionPatch::rollback_agent(Some(&old));
    assert_eq!(p.agent.as_deref(), Some("act"));
    assert!(!p.clear_agent);
    let p = SessionPatch::rollback_model(Some(&old));
    assert_eq!(p.model.as_deref(), Some("m"));
    assert!(!p.clear_model);

    // Captured row with NULL columns: clear (a plain None field is a no-op).
    let nulls = SessionMeta {
        id: "s".into(),
        ..Default::default()
    };
    let p = SessionPatch::rollback_agent(Some(&nulls));
    assert_eq!(p.agent, None);
    assert!(p.clear_agent, "NULL old agent must map to clear_agent");
    let p = SessionPatch::rollback_model(Some(&nulls));
    assert_eq!(p.model, None);
    assert!(p.clear_model, "NULL old model must map to clear_model");

    // Capture read failed (row unavailable): best-effort clear.
    let p = SessionPatch::rollback_agent(None);
    assert_eq!(p.agent, None);
    assert!(p.clear_agent, "unreadable old row must clear, not no-op");
    let p = SessionPatch::rollback_model(None);
    assert_eq!(p.model, None);
    assert!(p.clear_model, "unreadable old row must clear, not no-op");

    // Every rollback bumps updated_at (proves the write is observable).
    assert!(SessionPatch::rollback_agent(None).updated_at.is_some());
    assert!(SessionPatch::rollback_model(None).updated_at.is_some());
}
