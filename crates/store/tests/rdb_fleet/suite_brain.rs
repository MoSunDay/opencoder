//! Immutable brain plan versions, stable pointers and plan deletion.
use crate::common::{assignment, plan_version, prepare, request};
use opencoder_core::brain::PlanVersion;
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use opencoder_store::fleet::FleetStore;
use serde_json::{json, Value};

fn typed(document: &Value) -> PlanVersion {
    serde_json::from_value(document.clone()).expect("typed plan version")
}

async fn saved(store: &FleetStore, id: &str, version: u64, title: &str) -> Value {
    let document = plan_version(id, version, title);
    store
        .save_brain_plan_document(&serde_json::from_value(document.clone()).unwrap())
        .await
        .unwrap();
    document
}

#[tokio::test]
async fn saved_versions_are_readable_through_both_document_apis() {
    let store = FleetStore::open_memory().await.unwrap();
    let document = saved(&store, "plan-a", 1, "Plan A").await;

    let definition = store
        .save_brain_plan_document(&serde_json::from_value(document.clone()).unwrap())
        .await
        .unwrap();
    assert_eq!(definition.id, "plan-a");
    assert_eq!(definition.title, "Plan A");
    assert_eq!(definition.latest_version, 1);
    assert_eq!(definition.stable_version, None);
    assert_eq!(definition.updated_at, 1);

    let stored = store
        .brain_plan_document("plan-a", 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.version, 1);
    assert_eq!(stored.plan, document["plan"]);
    assert_eq!(stored.changelog, "v1");
    let typed_version = store
        .brain_plan_version("plan-a", 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(typed_version.version, 1);
    assert_eq!(typed_version.plan.title, "Plan A");
    assert_eq!(typed_version.changelog, "v1");

    assert!(store
        .brain_plan_document("plan-a", 2)
        .await
        .unwrap()
        .is_none());
    assert!(store
        .brain_plan_version("plan-a", 2)
        .await
        .unwrap()
        .is_none());
    assert!(store
        .brain_plan_document("plan-missing", 1)
        .await
        .unwrap()
        .is_none());

    let second = plan_version("plan-a", 2, "Plan B");
    let definition = store.save_brain_plan(&typed(&second)).await.unwrap();
    assert_eq!(definition.latest_version, 2);
    assert_eq!(definition.title, "Plan B");
    assert_eq!(definition.updated_at, 2);
}

#[tokio::test]
async fn version_publication_is_append_only_and_monotonic() {
    let store = FleetStore::open_memory().await.unwrap();
    let first = saved(&store, "plan-a", 1, "Original").await;

    let replayed = store
        .save_brain_plan_document(&serde_json::from_value(first.clone()).unwrap())
        .await
        .unwrap();
    assert_eq!(replayed.latest_version, 1, "exact replay is idempotent");

    let mut changed = first.clone();
    changed["plan"]["title"] = json!("Overwrite");
    let error = store
        .save_brain_plan_document(&serde_json::from_value(changed).unwrap())
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("immutable plan version conflict"),
        "{error}"
    );
    assert_eq!(
        store
            .brain_plan_document("plan-a", 1)
            .await
            .unwrap()
            .unwrap()
            .plan["title"],
        "Original"
    );

    let skipped = plan_version("plan-a", 3, "Skipped");
    assert!(store
        .save_brain_plan_document(&serde_json::from_value(skipped).unwrap())
        .await
        .is_err());
    assert!(store
        .brain_plan_document("plan-a", 3)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .definition("brain_plan", "plan-a")
            .await
            .unwrap()
            .unwrap()["latest_version"],
        1,
        "a rejected publication leaves the metadata untouched"
    );

    let mut zero = plan_version("plan-a", 0, "Zero");
    zero["version"] = json!(0);
    assert!(store
        .save_brain_plan_document(&serde_json::from_value(zero).unwrap())
        .await
        .is_err());

    let mut no_changelog = plan_version("plan-a", 2, "No changelog");
    no_changelog["changelog"] = json!("   ");
    assert!(store
        .save_brain_plan_document(&serde_json::from_value(no_changelog).unwrap())
        .await
        .is_err());

    let mut no_title = plan_version("plan-a", 2, "No title");
    no_title["plan"] = json!({"schema_version": 4, "objective": "test", "nodes": [], "edges": []});
    assert!(store
        .save_brain_plan_document(&serde_json::from_value(no_title).unwrap())
        .await
        .is_err());

    let invalid = plan_version("plan/invalid", 1, "Invalid");
    assert!(store
        .save_brain_plan_document(&serde_json::from_value(invalid).unwrap())
        .await
        .is_err());

    saved(&store, "plan-a", 2, "Second").await;
    assert_eq!(
        store
            .definition("brain_plan", "plan-a")
            .await
            .unwrap()
            .unwrap()["latest_version"],
        2
    );
}

#[tokio::test]
async fn listings_are_newest_first_and_honor_the_before_cursor() {
    let store = FleetStore::open_memory().await.unwrap();
    for version in 1..=3 {
        saved(&store, "plan-a", version, &format!("Plan {version}")).await;
    }

    let all = store.brain_plan_documents("plan-a", None).await.unwrap();
    assert_eq!(
        all.iter().map(|doc| doc.version).collect::<Vec<_>>(),
        [3, 2, 1]
    );
    let before_three = store.brain_plan_documents("plan-a", Some(3)).await.unwrap();
    assert_eq!(
        before_three
            .iter()
            .map(|doc| doc.version)
            .collect::<Vec<_>>(),
        [2, 1]
    );
    assert!(store
        .brain_plan_documents("plan-a", Some(1))
        .await
        .unwrap()
        .is_empty());
    let typed = store.brain_plan_versions("plan-a", Some(3)).await.unwrap();
    assert_eq!(
        typed.iter().map(|doc| doc.version).collect::<Vec<_>>(),
        [2, 1]
    );
    assert!(store
        .brain_plan_documents("plan-missing", None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn listings_never_leak_versions_from_other_rows() {
    let store = FleetStore::open_memory().await.unwrap();
    saved(&store, "plan-a", 1, "A1").await;
    saved(&store, "plan-a", 2, "A2").await;
    // Sorts before the queried prefix and must not be scanned into it.
    saved(&store, "plan-0", 1, "Other").await;
    // Frozen assignments occupy the key range below `bpv/`.
    let target = opencoder_core::fleet::ExecutionIndex {
        id: "agent-1".into(),
        created_at: 1,
        kind: ExecutionKind::Agent,
        node_id: "node-a".into(),
        status: ExecutionStatus::Pending,
    };
    let frozen = assignment(
        target,
        request("agent-1", ExecutionKind::Agent, json!({"prompt": "once"})),
        None,
    );
    prepare(&store, &frozen).await;

    let documents = store.brain_plan_documents("plan-a", None).await.unwrap();
    assert_eq!(
        documents
            .iter()
            .map(|doc| (doc.id.as_str(), doc.version))
            .collect::<Vec<_>>(),
        [("plan-a", 2), ("plan-a", 1)]
    );
    let other = store.brain_plan_documents("plan-0", None).await.unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].id, "plan-0");
}

#[tokio::test]
async fn mark_brain_stable_moves_the_pointer_and_reads_the_plan_row() {
    let store = FleetStore::open_memory().await.unwrap();
    saved(&store, "plan-a", 1, "One").await;
    saved(&store, "plan-a", 2, "Two").await;

    let definition = store.mark_brain_stable("plan-a", 1).await.unwrap();
    assert_eq!(definition.stable_version, Some(1));
    assert_eq!(definition.latest_version, 2);
    assert_eq!(
        store
            .definition("brain_plan", "plan-a")
            .await
            .unwrap()
            .unwrap()["stable_version"],
        1
    );

    saved(&store, "plan-a", 3, "Three").await;
    assert_eq!(
        store
            .definition("brain_plan", "plan-a")
            .await
            .unwrap()
            .unwrap()["stable_version"],
        1,
        "publishing later versions keeps the stable pointer"
    );

    let second = store.mark_brain_stable("plan-a", 2).await.unwrap();
    assert_eq!(second.stable_version, Some(2));
    let repeated = store.mark_brain_stable("plan-a", 2).await.unwrap();
    assert_eq!(repeated.stable_version, Some(2), "idempotent stable mark");

    let error = store.mark_brain_stable("plan-a", 9).await.unwrap_err();
    assert!(
        error.to_string().contains("unknown plan version"),
        "{error}"
    );
    assert!(store.mark_brain_stable("plan-missing", 1).await.is_err());
    assert_eq!(
        store
            .definition("brain_plan", "plan-a")
            .await
            .unwrap()
            .unwrap()["stable_version"],
        2
    );
}

#[tokio::test]
async fn delete_brain_plan_removes_versions_and_capability_metadata() {
    let store = FleetStore::open_memory().await.unwrap();
    saved(&store, "plan-a", 1, "One").await;
    saved(&store, "plan-a", 2, "Two").await;
    for id in ["plan-plan-a@1", "plan-plan-b@1"] {
        store
            .put_definition("brain_capability_meta", id, &json!({"maturity": "draft"}))
            .await
            .unwrap();
    }

    assert!(store.delete_brain_plan("plan-a").await.unwrap());
    assert!(store
        .brain_plan_document("plan-a", 1)
        .await
        .unwrap()
        .is_none());
    assert!(store
        .brain_plan_documents("plan-a", None)
        .await
        .unwrap()
        .is_empty());
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
    assert!(
        store
            .definition("brain_capability_meta", "plan-plan-b@1")
            .await
            .unwrap()
            .is_some(),
        "other plans keep their capability metadata"
    );
    assert!(
        !store.delete_brain_plan("plan-a").await.unwrap(),
        "missing plans report false"
    );
    assert!(store.delete_brain_plan("plan/invalid").await.is_err());
}
