//! Summary fields: independent timestamps, clear-without-touching-progress,
//! ten-version history windows, restore and reopen persistence (ported from
//! the SQL summary suites; the SQL history-write fault injection stays
//! SQL-only — rdb writes are staged in one batch, so there is no partial
//! history row to inject against).
use super::*;
use opencoder_store::project::overview;
use opencoder_store::project::summary::SummaryScope;
use serde_json::json;

async fn seed(s: &dyn ProjectStore) {
    s.create_goal(&goal("p", 0, 1)).await.unwrap();
    s.create_initiative(&initiative("i", Some("p"), 0, 1))
        .await
        .unwrap();
}

fn summary_patch(text: &str) -> opencoder_store::ProjectGoalPatch {
    serde_json::from_value(json!({"current_summary":text})).unwrap()
}

#[tokio::test]
async fn summaries_have_independent_timestamps_and_clear_without_changing_progress() {
    let s = mem_iface().await;
    seed(s.as_ref()).await;
    assert!(s.list_goals().await.unwrap()[0]
        .summary
        .summary_updated_at
        .is_none());
    s.patch_goal("p", &summary_patch(" 关键进展：接口完成 "), 10)
        .await
        .unwrap();
    s.patch_initiative(
        "i",
        &serde_json::from_value::<opencoder_store::ProjectInitiativePatch>(
            json!({"current_summary":"卡点：待提供测试设备"}),
        )
        .unwrap(),
        20,
    )
    .await
    .unwrap();
    // A later unrelated change stamps updated_at but not the summary clock.
    s.patch_goal(
        "p",
        &serde_json::from_value(json!({"title":"Renamed"})).unwrap(),
        30,
    )
    .await
    .unwrap();
    let g = s.list_goals().await.unwrap().remove(0);
    assert_eq!(g.summary.current_summary, "关键进展：接口完成");
    assert_eq!(g.summary.summary_updated_at, Some(10));
    assert_eq!(g.updated_at, 30);
    let i = s.list_initiatives(None).await.unwrap().remove(0);
    assert_eq!(i.summary.summary_updated_at, Some(20));
    assert!(serde_json::to_value(&i).unwrap().get("status").is_none());
    // Summary and owner changes share one transaction.
    assert!(s
        .patch_goal(
            "p",
            &serde_json::from_value(
                json!({"current_summary":"invalid", "owner_user_name":"missing"})
            )
            .unwrap(),
            40,
        )
        .await
        .is_err());
    assert_eq!(
        s.list_goals().await.unwrap()[0].summary.summary_updated_at,
        Some(10)
    );
    // Clearing the initiative summary keeps it visible as "".
    s.patch_initiative(
        "i",
        &serde_json::from_value::<opencoder_store::ProjectInitiativePatch>(
            json!({"current_summary":""}),
        )
        .unwrap(),
        50,
    )
    .await
    .unwrap();
    let snapshot = overview::load(s.as_ref()).await.unwrap();
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
    assert!(!s
        .patch_goal("missing", &Default::default(), 60)
        .await
        .unwrap());
}

#[tokio::test]
async fn summary_history_retains_ten_versions_and_restore_appends_without_touching_other_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history-db");
    let s = RdbStore::open(&path).await.unwrap();
    seed(&s).await;
    for n in 1..=12 {
        s.patch_goal("p", &summary_patch(&format!("revision {n}")), n * 10)
            .await
            .unwrap();
        s.patch_initiative(
            "i",
            &serde_json::from_value::<opencoder_store::ProjectInitiativePatch>(
                json!({"current_summary":format!("initiative {n}")}),
            )
            .unwrap(),
            n * 10,
        )
        .await
        .unwrap();
    }
    // Ten most recent versions, newest first; older ones fall off.
    let history = s
        .summary_versions(SummaryScope::Project, "p")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.len(), 10);
    assert_eq!(history[0].version, 13);
    assert_eq!(history[9].version, 4);
    s.patch_goal(
        "p",
        &serde_json::from_value(json!({"title":"Renamed"})).unwrap(),
        150,
    )
    .await
    .unwrap();
    // A rolled-back patch leaves no history row.
    assert!(s
        .patch_goal(
            "p",
            &serde_json::from_value(
                json!({"current_summary":"invalid","owner_user_name":"missing"})
            )
            .unwrap(),
            160,
        )
        .await
        .is_err());
    assert_eq!(
        s.summary_versions(SummaryScope::Project, "p")
            .await
            .unwrap()
            .unwrap()[0]
            .version,
        13
    );
    // Restore appends a new version pointing at the restored one.
    for (scope, id, expected) in [
        (SummaryScope::Project, "p", "revision 3"),
        (SummaryScope::Initiative, "i", "initiative 3"),
    ] {
        let restored = s.restore_summary(scope, id, 4, 200).await.unwrap().unwrap();
        assert_eq!(restored.version, 14);
        assert_eq!(restored.restored_from, Some(4));
        assert_eq!(restored.current_summary, expected);
        assert_eq!(restored.created_at, 200);
        // Unknown version: None, no history growth.
        assert!(s
            .restore_summary(scope, id, 4, 210)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            s.summary_versions(scope, id).await.unwrap().unwrap().len(),
            10
        );
    }
    let g = s.list_goals().await.unwrap().remove(0);
    assert_eq!(g.title, "Renamed");
    assert_eq!(g.summary.current_summary, "revision 3");
    assert_eq!(g.summary.summary_updated_at, Some(200));
    let i = s.list_initiatives(None).await.unwrap().remove(0);
    assert_eq!(i.goal_id.as_deref(), Some("p"));
    assert!(serde_json::to_value(&i).unwrap().get("status").is_none());
    s.patch_goal("p", &summary_patch(""), 220).await.unwrap();

    // Reopen: history and summaries survive the restart.
    drop(s);
    let s = RdbStore::open(&path).await.unwrap();
    let history = s
        .summary_versions(SummaryScope::Project, "p")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.len(), 10);
    assert_eq!(history[0].version, 15);
    assert_eq!(history[0].current_summary, "");
    assert_eq!(history[1].restored_from, Some(4));
    // Cross-scope lookups are None.
    assert!(s
        .summary_versions(SummaryScope::Initiative, "p")
        .await
        .unwrap()
        .is_none());
    // Deleting the goal drops its history but not the initiative's.
    s.delete_goal("p").await.unwrap();
    assert!(s
        .summary_versions(SummaryScope::Project, "p")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        s.summary_versions(SummaryScope::Initiative, "i")
            .await
            .unwrap()
            .unwrap()
            .len(),
        10
    );
    // Re-created objects start a fresh history at version 1.
    s.delete_initiative("i").await.unwrap();
    seed(&s).await;
    for (scope, id) in [
        (SummaryScope::Project, "p"),
        (SummaryScope::Initiative, "i"),
    ] {
        let history = s.summary_versions(scope, id).await.unwrap().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].version, 1);
        assert_eq!(history[0].current_summary, "");
    }
}
