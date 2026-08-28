//! Regression test for the legacy agent-name normalization (plan/act split
//! removal). Databases written before the refactor may still store
//! `agent = 'plan'` for the read-only agent, which is now named `sandbox`.
//! Resume must treat those rows as act sessions: every store READ path maps
//! the stored `'plan'` to `'act'`, while the raw row is never rewritten.

use opencoder_store::{LibsqlStore, SessionFilter, SessionMeta, Store};

#[tokio::test]
async fn legacy_plan_agent_reads_back_as_act_on_all_read_paths() {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();

    store
        .create_session(&SessionMeta {
            id: "legacy".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // Simulate a database written before the refactor: raw-UPDATE the agent
    // column straight to the legacy value, bypassing the typed API.
    {
        let conn = store.conn().await.unwrap();
        let updated = conn
            .execute("UPDATE sessions SET agent = 'plan' WHERE id = 'legacy'", ())
            .await
            .unwrap();
        assert_eq!(updated, 1, "exactly one row must carry the legacy value");
    }

    // (1) get_session normalizes: meta reads 'act' ...
    let meta = store
        .get_session("legacy")
        .await
        .unwrap()
        .expect("session exists");
    assert_eq!(
        meta.agent.as_deref(),
        Some("act"),
        "get_session must map legacy agent 'plan' to 'act' so old sessions resume"
    );

    // ... while the stored row is untouched.
    {
        let conn = store.conn().await.unwrap();
        let mut rows = conn
            .query("SELECT agent FROM sessions WHERE id = 'legacy'", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().expect("raw row exists");
        let raw: String = row.get(0).unwrap();
        assert_eq!(
            raw, "plan",
            "normalization is read-only: the stored agent must stay 'plan'"
        );
    }

    // (2) list_sessions shares the same normalization.
    let items = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    assert_eq!(items.len(), 1, "exactly one session listed");
    assert_eq!(
        items[0].agent.as_deref(),
        Some("act"),
        "list_sessions must map legacy agent 'plan' to 'act'"
    );

    // (3) Non-legacy values pass through untouched.
    store
        .create_session(&SessionMeta {
            id: "sandboxed".into(),
            agent: Some("sandbox".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let modern = store.get_session("sandboxed").await.unwrap().unwrap();
    assert_eq!(
        modern.agent.as_deref(),
        Some("sandbox"),
        "'sandbox' must not be rewritten by the legacy mapping"
    );
}
