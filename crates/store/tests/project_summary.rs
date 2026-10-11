use opencoder_store::project::summary::SummaryScope;
use opencoder_store::{LibsqlStore, ProjectStore};
use serde_json::json;

#[path = "project_summary/support.rs"]
mod support;
use support::seed;

#[tokio::test]
async fn summaries_have_independent_timestamps_and_clear_without_changing_progress() {
    let store = LibsqlStore::open_memory().await.unwrap();
    seed(&store).await;
    assert!(store.list_goals().await.unwrap()[0]
        .summary
        .summary_updated_at
        .is_none());
    store
        .patch_goal(
            "p",
            &serde_json::from_value(json!({"current_summary":" 关键进展：接口完成 "})).unwrap(),
            10,
        )
        .await
        .unwrap();
    store
        .patch_initiative(
            "i",
            &serde_json::from_value(json!({"current_summary":"卡点：待提供测试设备"})).unwrap(),
            20,
        )
        .await
        .unwrap();
    store
        .patch_goal(
            "p",
            &serde_json::from_value(json!({"title":"Renamed"})).unwrap(),
            30,
        )
        .await
        .unwrap();
    let goal = store.list_goals().await.unwrap().remove(0);
    assert_eq!(goal.summary.current_summary, "关键进展：接口完成");
    assert_eq!(goal.summary.summary_updated_at, Some(10));
    assert_eq!(goal.updated_at, 30);
    let initiative = store.list_initiatives(None).await.unwrap().remove(0);
    assert_eq!(initiative.summary.summary_updated_at, Some(20));
    assert!(serde_json::to_value(&initiative)
        .unwrap()
        .get("status")
        .is_none());
    // Summary and owner changes share the existing transaction.
    assert!(store
        .patch_goal(
            "p",
            &serde_json::from_value(
                json!({"current_summary":"invalid", "owner_user_name":"missing"})
            )
            .unwrap(),
            40
        )
        .await
        .is_err());
    assert_eq!(
        store.list_goals().await.unwrap()[0]
            .summary
            .summary_updated_at,
        Some(10)
    );
    store
        .patch_initiative(
            "i",
            &serde_json::from_value(json!({"current_summary":""})).unwrap(),
            50,
        )
        .await
        .unwrap();
    let snapshot = opencoder_store::project::overview::load(&store)
        .await
        .unwrap();
    assert_eq!(
        snapshot["goals"][0]["current_summary"],
        "关键进展：接口完成"
    );
    assert_eq!(
        snapshot["goals"][0]["initiatives"][0]["current_summary"],
        ""
    );
    assert_eq!(
        snapshot["goals"][0]["initiatives"][0]["summary_updated_at"],
        50
    );
    assert_eq!(
        snapshot["goals"][0]["progress"],
        json!({"total":0,"done":0})
    );
    assert!(!store
        .patch_goal("missing", &Default::default(), 60)
        .await
        .unwrap());
}

#[tokio::test]
async fn schema_36_upgrade_preserves_records_and_reopen_preserves_summaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project.db");
    let store = LibsqlStore::open(&path).await.unwrap();
    seed(&store).await;
    drop(store);
    let db = libsql::Builder::new_local(&path).build().await.unwrap();
    let conn = db.connect().unwrap();
    for table in ["project_goals", "project_initiatives"] {
        conn.execute(
            &format!("ALTER TABLE {table} DROP COLUMN current_summary"),
            (),
        )
        .await
        .unwrap();
        conn.execute(
            &format!("ALTER TABLE {table} DROP COLUMN summary_updated_at"),
            (),
        )
        .await
        .unwrap();
    }
    conn.execute("UPDATE schema_version SET version=36", ())
        .await
        .unwrap();
    conn.execute("DROP TABLE project_summary_versions", ())
        .await
        .unwrap();
    drop(conn);
    drop(db);
    let store = LibsqlStore::open(&path).await.unwrap();
    let goal = store.list_goals().await.unwrap().remove(0);
    assert_eq!(goal.title, "Project");
    assert_eq!(goal.summary.current_summary, "");
    assert_eq!(goal.summary.summary_updated_at, None);
    assert_eq!(
        store
            .summary_versions(SummaryScope::Project, "p")
            .await
            .unwrap()
            .unwrap()[0]
            .current_summary,
        ""
    );
    store
        .patch_goal(
            "p",
            &serde_json::from_value(json!({"current_summary":"saved"})).unwrap(),
            70,
        )
        .await
        .unwrap();
    drop(store);
    let store = LibsqlStore::open(&path).await.unwrap();
    assert_eq!(
        store.list_goals().await.unwrap()[0].summary.current_summary,
        "saved"
    );
    assert_eq!(
        store.list_initiatives(None).await.unwrap()[0]
            .goal_id
            .as_deref(),
        Some("p")
    );
}
