use super::support::{binary, call, Harness};
use serde_json::{json, Value};

#[tokio::test]
async fn binary_duplicate_uploads_do_not_create_pools_or_versions() {
    let h = Harness::new().await;
    assert_eq!(
        h.call("POST", "/api/dag/binaries", binary("tool", "one"))
            .await
            .0,
        201
    );
    let before = h.call("GET", "/api/dag/binaries/tool", Value::Null).await;
    let retry = h
        .call("PUT", "/api/dag/binaries/tool", binary("tool", "one"))
        .await;
    assert_eq!(retry.0, 200, "{}", retry.1);
    assert_eq!(retry.1["unchanged"], true);
    assert_eq!(
        h.call("GET", "/api/dag/binaries/tool", Value::Null).await.1,
        before.1
    );
    let duplicate = h
        .call("POST", "/api/dag/binaries/copy", binary("copy", "one"))
        .await;
    assert_eq!(duplicate.0, 405);
    let duplicate = h
        .call("POST", "/api/dag/binaries", binary("copy", "one"))
        .await;
    assert_eq!(duplicate.0, 409, "{}", duplicate.1);
    assert_eq!(duplicate.1["details"]["name"], "tool");
    assert_eq!(
        h.call("PUT", "/api/dag/binaries/tool", binary("tool", "two"))
            .await
            .1["version"],
        2
    );
    assert_eq!(
        h.call("PUT", "/api/dag/binaries/tool", binary("tool", "one"))
            .await
            .0,
        409
    );
    assert_eq!(
        h.call(
            "POST",
            "/api/dag/binaries/tool/rollback",
            json!({"version":1})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        h.call("GET", "/api/dag/binaries/tool", Value::Null).await.1["history"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn resource_delete_checks_etag_and_all_versions_of_named_references() {
    let h = Harness::new().await;
    h.call("POST", "/api/dag/binaries", binary("tool", "one"))
        .await;
    let before = h.call("GET", "/api/dag/binaries/tool", Value::Null).await;
    assert!(!before.2.is_empty());
    let spec = json!({"name":"consumer","steps":[{"name":"run","kind":{"type":"binary","resource":"tool@v1"}}]});
    assert_eq!(h.call("POST", "/api/dag/defs", spec).await.0, 200);
    let refused = h
        .call("DELETE", "/api/dag/binaries/tool", Value::Null)
        .await;
    assert_eq!(refused.0, 409, "{}", refused.1);
    assert!(h.dir.path().join("binaries/tool/v1/binary.bin").exists());
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/consumer", Value::Null)
            .await
            .0,
        200
    );
    h.call("PUT", "/api/dag/binaries/tool", binary("tool", "two"))
        .await;
    assert_eq!(
        call(
            &h.app,
            "DELETE",
            "/api/dag/binaries/tool",
            Value::Null,
            Some(&before.2)
        )
        .await
        .0,
        412
    );
    assert_eq!(
        h.call("DELETE", "/api/dag/binaries/tool", Value::Null)
            .await
            .0,
        200
    );
    assert!(!h.dir.path().join("binaries/tool").exists());
    let late = json!({"name":"late","steps":[{"name":"run","kind":{"type":"binary","resource":"tool@v1"}}]});
    let rejected = h.call("POST", "/api/dag/defs", late).await;
    assert_eq!(rejected.0, 409, "{}", rejected.1);
}

#[tokio::test]
async fn concurrent_uploads_with_different_names_create_one_pool() {
    let h = Harness::new().await;
    let (a, b) = tokio::join!(
        h.call("POST", "/api/dag/binaries", binary("one", "same")),
        h.call("POST", "/api/dag/binaries", binary("two", "same"))
    );
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [201, 409], "{},{}", a.1, b.1);
    assert_eq!(
        h.call("GET", "/api/dag/binaries", Value::Null).await.1["pools"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
