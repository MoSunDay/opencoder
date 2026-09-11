use opencoder_core::brain::*;
use opencoder_store::fleet::FleetStore;
use serde_json::json;

fn version() -> PlanVersion {
    serde_json::from_value(json!({"id":"plan-a","version":1,"plan":{"schema_version":1,"title":"A","objective":"test","steps":[],"deliverables":{}},"changelog":"initial","created_at":1})).unwrap()
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
async fn cross_run_claims_allow_shared_reads_and_require_real_release_before_write() {
    let store = FleetStore::open_memory().await.unwrap();
    let read = vec![ResourceUse {
        key: "repo:main".into(),
        mode: AccessMode::Read,
    }];
    let write = vec![ResourceUse {
        key: "repo:main".into(),
        mode: AccessMode::Write,
    }];
    assert!(store
        .claim_brain_resources("agent-a", "brain-a", &read)
        .await
        .unwrap());
    assert!(store
        .claim_brain_resources("agent-b", "brain-b", &read)
        .await
        .unwrap());
    assert!(!store
        .claim_brain_resources("agent-c", "brain-c", &write)
        .await
        .unwrap());
    let waiters = store.release_brain_resources("agent-a").await.unwrap();
    assert_eq!(waiters, vec!["brain-c"]);
    assert!(!store
        .claim_brain_resources("agent-c", "brain-c", &write)
        .await
        .unwrap());
    store.release_brain_resources("agent-b").await.unwrap();
    assert!(store
        .claim_brain_resources("agent-c", "brain-c", &write)
        .await
        .unwrap());
    assert!(store
        .claim_brain_resources("agent-c", "brain-c", &write)
        .await
        .unwrap());
    assert!(store.brain_resource_claims("brain-a").await.unwrap()[0].released);
    assert!(store
        .claim_brain_resources("agent-c", "brain-c", &read)
        .await
        .is_err());
}
