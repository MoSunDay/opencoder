use opencoder_store::{project::summary::SummaryScope, ProjectStore, TursoStore};
use serde_json::json;
#[path = "project_summary/support.rs"]
mod support;
use support::seed;

#[tokio::test]
async fn summary_history_retains_ten_versions_and_restore_appends_without_touching_other_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let store = TursoStore::open(&path).await.unwrap();
    seed(&store).await;
    for n in 1..=12 {
        store
            .patch_goal(
                "p",
                &serde_json::from_value(json!({"current_summary":format!("revision {n}")}))
                    .unwrap(),
                n * 10,
            )
            .await
            .unwrap();
        store
            .patch_initiative(
                "i",
                &serde_json::from_value(json!({"current_summary":format!("initiative {n}")}))
                    .unwrap(),
                n * 10,
            )
            .await
            .unwrap();
    }
    let history = store
        .summary_versions(SummaryScope::Project, "p")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.len(), 10);
    assert_eq!(history[0].version, 13);
    assert_eq!(history[9].version, 4);
    store
        .patch_goal(
            "p",
            &serde_json::from_value(json!({"title":"Renamed"})).unwrap(),
            150,
        )
        .await
        .unwrap();
    assert!(store
        .patch_goal(
            "p",
            &serde_json::from_value(
                json!({"current_summary":"invalid","owner_user_name":"missing"})
            )
            .unwrap(),
            160
        )
        .await
        .is_err());
    assert_eq!(
        store
            .summary_versions(SummaryScope::Project, "p")
            .await
            .unwrap()
            .unwrap()[0]
            .version,
        13
    );
    for (scope, id, expected) in [
        (SummaryScope::Project, "p", "revision 3"),
        (SummaryScope::Initiative, "i", "initiative 3"),
    ] {
        let restored = store
            .restore_summary(scope, id, 4, 200)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restored.version, 14);
        assert_eq!(restored.restored_from, Some(4));
        assert_eq!(restored.current_summary, expected);
        assert_eq!(restored.created_at, 200);
        assert!(store
            .restore_summary(scope, id, 4, 210)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .summary_versions(scope, id)
                .await
                .unwrap()
                .unwrap()
                .len(),
            10
        );
    }
    let goal = store.list_goals().await.unwrap().remove(0);
    assert_eq!(goal.title, "Renamed");
    assert_eq!(goal.summary.current_summary, "revision 3");
    assert_eq!(goal.summary.summary_updated_at, Some(200));
    let initiative = store.list_initiatives(None).await.unwrap().remove(0);
    assert_eq!(initiative.goal_id.as_deref(), Some("p"));
    assert!(serde_json::to_value(&initiative)
        .unwrap()
        .get("status")
        .is_none());
    store
        .patch_goal(
            "p",
            &serde_json::from_value(json!({"current_summary":""})).unwrap(),
            220,
        )
        .await
        .unwrap();
    drop(store);
    let store = TursoStore::open(&path).await.unwrap();
    let history = store
        .summary_versions(SummaryScope::Project, "p")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.len(), 10);
    assert_eq!(history[0].version, 15);
    assert_eq!(history[0].current_summary, "");
    assert_eq!(history[1].restored_from, Some(4));
    assert!(store
        .summary_versions(SummaryScope::Initiative, "p")
        .await
        .unwrap()
        .is_none());
    store.delete_goal("p").await.unwrap();
    assert!(store
        .summary_versions(SummaryScope::Project, "p")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .summary_versions(SummaryScope::Initiative, "i")
            .await
            .unwrap()
            .unwrap()
            .len(),
        10
    );
    store.delete_initiative("i").await.unwrap();
    seed(&store).await;
    for (scope, id) in [
        (SummaryScope::Project, "p"),
        (SummaryScope::Initiative, "i"),
    ] {
        let history = store.summary_versions(scope, id).await.unwrap().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].version, 1);
        assert_eq!(history[0].current_summary, "");
    }
}

#[tokio::test]
async fn history_write_failure_rolls_back_summary_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("atomic.db");
    let store = TursoStore::open(&path).await.unwrap();
    seed(&store).await;
    store
        .patch_goal(
            "p",
            &serde_json::from_value(json!({"current_summary":"saved"})).unwrap(),
            10,
        )
        .await
        .unwrap();
    let db = turso::Builder::new_local(path.to_str().expect("UTF-8 database path"))
        .experimental_multiprocess_wal(true)
        .build()
        .await
        .unwrap();
    db.connect().unwrap().execute("CREATE TRIGGER fail_history BEFORE INSERT ON project_summary_versions BEGIN SELECT RAISE(ABORT, 'history failed'); END", ()).await.unwrap();
    assert!(store
        .patch_goal(
            "p",
            &serde_json::from_value(json!({"title":"bad","current_summary":"bad"})).unwrap(),
            20
        )
        .await
        .is_err());
    assert!(store
        .restore_summary(SummaryScope::Project, "p", 1, 30)
        .await
        .is_err());
    let goal = store.list_goals().await.unwrap().remove(0);
    assert_eq!(goal.title, "Project");
    assert_eq!(goal.summary.current_summary, "saved");
    assert_eq!(goal.summary.summary_updated_at, Some(10));
    assert_eq!(
        store
            .summary_versions(SummaryScope::Project, "p")
            .await
            .unwrap()
            .unwrap()
            .len(),
        2
    );
}
