//! Regression test for the legacy agent-name normalization (sandbox-mode
//! interlude revert). Databases written during the interlude may still store
//! `agent = 'sandbox'` for the read-only agent, which is named `plan` again.
//! Every store READ path maps the stored `'sandbox'` to `'plan'`, while the
//! raw row is never rewritten.

use opencoder_store::{LibsqlStore, SessionFilter, SessionMeta, Store};

#[tokio::test]
async fn legacy_sandbox_agent_reads_back_as_plan_on_all_read_paths() {
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
            .execute(
                "UPDATE sessions SET agent = 'sandbox' WHERE id = 'legacy'",
                (),
            )
            .await
            .unwrap();
        assert_eq!(updated, 1, "exactly one row must carry the legacy value");
    }

    // (1) get_session normalizes: meta reads 'plan' ...
    let meta = store
        .get_session("legacy")
        .await
        .unwrap()
        .expect("session exists");
    assert_eq!(
        meta.agent.as_deref(),
        Some("plan"),
        "get_session must map legacy agent 'sandbox' to 'plan' so old sessions resume"
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
            raw, "sandbox",
            "normalization is read-only: the stored agent must stay 'sandbox'"
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
        Some("plan"),
        "list_sessions must map legacy agent 'sandbox' to 'plan'"
    );

    // (3) Non-legacy values pass through untouched.
    store
        .create_session(&SessionMeta {
            id: "modern".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let modern = store.get_session("modern").await.unwrap().unwrap();
    assert_eq!(
        modern.agent.as_deref(),
        Some("act"),
        "modern values must not be rewritten by the legacy mapping"
    );
}
