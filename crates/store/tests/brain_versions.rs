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
