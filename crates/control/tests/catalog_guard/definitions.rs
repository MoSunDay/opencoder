use super::support::{call, spec, Harness};
use serde_json::{json, Value};

#[tokio::test]
async fn equivalent_names_conflict_while_identical_retries_preserve_timestamps() {
    let h = Harness::new().await;
    let a = h.call("POST", "/api/dag/defs", spec("first")).await;
    assert_eq!(a.0, 200, "{:?}", a.1);
    let retry = h.call("POST", "/api/dag/defs", spec("first")).await;
    assert_eq!(retry.1, a.1);
    let duplicate = h.call("POST", "/api/dag/defs", spec("second")).await;
    assert_eq!(duplicate.0, 409, "{:?}", duplicate.1);
    assert_eq!(duplicate.1["details"]["name"], "first");
    let mut changed = spec("second");
    changed["steps"][0]["kind"]["prompt"] = json!("different task");
    assert_eq!(h.call("POST", "/api/dag/defs", changed).await.0, 200);
}

#[tokio::test]
async fn concurrent_servers_publish_one_equivalent_definition() {
    let h = Harness::new().await;
    let second =
        opencoder_control::new_state(h.state.workdir.clone(), h.dir.path().join("state"), None)
            .await
            .unwrap();
    let app = opencoder_control::build_app(second, None, false);
    let (a, b) = tokio::join!(
        h.call("POST", "/api/dag/defs", spec("first")),
        call(&app, "POST", "/api/dag/defs", spec("second"), None)
    );
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    assert_eq!(h.state.fleet.definitions("dag").await.unwrap().len(), 1);
}

#[tokio::test]
async fn conditional_removal_rechecks_contents_and_preserves_execution_history() {
    let h = Harness::new().await;
    h.call("POST", "/api/dag/defs", spec("first")).await;
    let before = h.call("GET", "/api/dag/defs/first", Value::Null).await;
    assert!(!before.2.is_empty());
    let mut changed = spec("first");
    changed["description"] = json!("new description");
    h.call("POST", "/api/dag/defs", changed).await;
    assert_eq!(
        call(
            &h.app,
            "DELETE",
            "/api/dag/defs/first",
            Value::Null,
            Some(&before.2)
        )
        .await
        .0,
        412
    );
    let current = h.call("GET", "/api/dag/defs/first", Value::Null).await;
    assert_eq!(
        call(
            &h.app,
            "DELETE",
            "/api/dag/defs/first",
            Value::Null,
            Some(&current.2)
        )
        .await
        .0,
        200
    );
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/first", Value::Null).await.0,
        404
    );
    assert!(h
        .state
        .fleet
        .definition("catalog_retired", "dag:first")
        .await
        .unwrap()
        .is_some());
}
