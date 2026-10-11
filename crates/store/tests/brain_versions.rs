use opencoder_core::brain::*;
use opencoder_store::fleet::FleetStore;
use serde_json::json;

fn version() -> PlanVersion {
    serde_json::from_value(json!({"id":"plan-a","version":1,"plan":{"schema_version":4,"title":"A","objective":"test","nodes":[],"edges":[]},"changelog":"initial","created_at":1})).unwrap()
}
#[tokio::test]
async fn versions_are_append_only_conflicts_do_not_move_stable_pointer() {
    let store = FleetStore::open_memory().await.unwrap();
    let mut first = version();
    store.save_brain_plan(&first).await.unwrap();
    store.save_brain_plan(&first).await.unwrap();
    store.mark_brain_stable("plan-a", 1).await.unwrap();
    first.plan.title = "overwrite".into();
    assert!(store.save_brain_plan(&first).await.is_err());
    first.version = 2;
    first.changelog = "new title".into();
    store.save_brain_plan(&first).await.unwrap();
    assert_eq!(
        store
            .brain_plan_version("plan-a", 1)
            .await
            .unwrap()
            .unwrap()
            .plan
            .title,
        "A"
    );
    let meta = store
        .definition("brain_plan", "plan-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta["stable_version"], 1);
    assert_eq!(meta["latest_version"], 2);
    assert_eq!(
        store
            .brain_plan_versions("plan-a", Some(2))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(store.mark_brain_stable("plan-a", 3).await.is_err());
}

#[tokio::test]
async fn deleting_brain_plan_removes_versions_and_catalog_metadata() {
    let store = FleetStore::open_memory().await.unwrap();
    let first = version();
    store.save_brain_plan(&first).await.unwrap();
    store
        .put_definition(
            "brain_capability_meta",
            "plan-plan-a@1",
            &json!({"maturity":"draft"}),
        )
        .await
        .unwrap();

    assert!(store.delete_brain_plan("plan-a").await.unwrap());
    assert!(store
        .brain_plan_document("plan-a", 1)
        .await
        .unwrap()
        .is_none());
    assert!(store
        .definition("brain_plan", "plan-a")
        .await
        .unwrap()
        .is_none());
    assert!(store
        .definition("brain_capability_meta", "plan-plan-a@1")
        .await
        .unwrap()
        .is_none());
    assert!(!store.delete_brain_plan("plan-a").await.unwrap());
}

#[tokio::test]
async fn deleting_plan_uses_exact_case_sensitive_prefix_and_all_versions() {
    let store = FleetStore::open_memory().await.unwrap();
    let mut first = version();
    first.id = "plan_a".into();
    store.save_brain_plan(&first).await.unwrap();
    first.version = 2;
    store.save_brain_plan(&first).await.unwrap();
    for id in [
        "plan-plan_a@1",
        "plan-plan_a@2",
        "plan-planXa@1",
        "plan-Plan_a@1",
    ] {
        store
            .put_definition("brain_capability_meta", id, &json!({"maturity":"stable"}))
            .await
            .unwrap();
    }
    assert!(store.delete_brain_plan("plan_a").await.unwrap());
    assert!(store
        .brain_plan_documents("plan_a", None)
        .await
        .unwrap()
        .is_empty());
    for id in ["plan-plan_a@1", "plan-plan_a@2"] {
        assert!(store
            .definition("brain_capability_meta", id)
            .await
            .unwrap()
            .is_none());
    }
    for id in ["plan-planXa@1", "plan-Plan_a@1"] {
        assert!(store
            .definition("brain_capability_meta", id)
            .await
            .unwrap()
            .is_some());
    }
    assert!(store.delete_brain_plan("invalid/id").await.is_err());
}
