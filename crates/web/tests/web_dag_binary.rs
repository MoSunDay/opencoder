//! `/api/dag/binaries` happy-path contract tests (create/version/rollback/
//! download/delete). Harness in `support/dag_binary.rs`; error contracts
//! (400/404) live in `web_dag_binary_errors.rs`.

mod support;

use axum::http::StatusCode;

use support::dag_binary::{
    app, b64, call, call_raw, create, scoped, sha256_hex, state, MODULE, MODULE_V2,
};

#[tokio::test]
async fn create_writes_v1_meta_binary_and_digest() {
    let scoped_pair = scoped();
    let dir = scoped_pair.0;
    let router = app(state().await);
    let (status, v) = call(
        router.clone(),
        "POST",
        "/api/dag/binaries",
        Some(serde_json::json!({
            "name": "adder",
            "description": "first cut",
            "binary_b64": b64(MODULE),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["name"], "adder");
    assert_eq!(v["version"], 1);

    let root = dir.path();
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("adder/meta.json")).unwrap())
            .unwrap();
    assert_eq!(meta["current"], 1);
    assert_eq!(meta["history"], serde_json::json!([1]));
    assert_eq!(meta["name"], "adder");
    assert_eq!(meta["description"], "first cut");
    assert_eq!(
        std::fs::read(root.join("adder/v1/binary.bin")).unwrap(),
        MODULE.to_vec()
    );
    let vmeta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("adder/v1/meta.json")).unwrap())
            .unwrap();
    assert_eq!(vmeta["version"], 1);
    assert_eq!(vmeta["sha256"], sha256_hex(MODULE));
    assert_eq!(vmeta["size_bytes"], MODULE.len() as u64);

    // The same facts come back through the read API.
    let (status, v) = call(router, "GET", "/api/dag/binaries/adder", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["current"], 1);
    assert_eq!(v["history"][0]["sha256"], sha256_hex(MODULE));
}

#[tokio::test]
async fn duplicate_create_conflicts() {
    let _scoped = scoped();
    let router = app(state().await);
    create(router.clone(), "adder").await;
    let (status, v) = call(
        router,
        "POST",
        "/api/dag/binaries",
        Some(serde_json::json!({ "name": "adder", "binary_b64": b64(MODULE) })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap_or("").contains("already exists"));
}

#[tokio::test]
async fn put_rolls_versions_and_rollback_never_reuses() {
    let _scoped = scoped();
    let router = app(state().await);
    create(router.clone(), "adder").await;

    // PUT new bytes ⇒ v2, now current.
    let (status, v) = call(
        router.clone(),
        "PUT",
        "/api/dag/binaries/adder",
        Some(serde_json::json!({
            "description": "second cut",
            "binary_b64": b64(MODULE_V2),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["version"], 2);

    let (status, v) = call(router.clone(), "GET", "/api/dag/binaries/adder", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["current"], 2);
    assert_eq!(
        v["history"]
            .as_array()
            .map(|h| h.iter().map(|m| m["version"].clone()).collect::<Vec<_>>()),
        Some(vec![serde_json::json!(1), serde_json::json!(2)])
    );

    // Rollback to v1 ⇒ pointer-only switch.
    let (status, v) = call(
        router.clone(),
        "POST",
        "/api/dag/binaries/adder/rollback",
        Some(serde_json::json!({ "version": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["version"], 1);
    let (_status, v) = call(router.clone(), "GET", "/api/dag/binaries/adder", None).await;
    assert_eq!(v["current"], 1);

    // PUT again ⇒ v3 (max(history)+1), never reusing v2.
    let (status, v) = call(
        router.clone(),
        "PUT",
        "/api/dag/binaries/adder",
        Some(serde_json::json!({
            "description": "third cut",
            "binary_b64": b64(MODULE_V2),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        v["version"], 3,
        "versioning must stay monotonic across rollback"
    );

    // Download v3 ⇒ full binary with octet-stream headers.
    let (status, headers, bytes) = call_raw(
        router,
        "GET",
        "/api/dag/binaries/adder/versions/3/binary.bin",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("content-type").unwrap(),
        "application/octet-stream"
    );
    assert_eq!(
        headers.get("content-disposition").unwrap(),
        "attachment; filename=\"adder-v3.binary\""
    );
    assert_eq!(
        headers.get("content-length").unwrap(),
        MODULE_V2.len().to_string().as_str()
    );
    assert_eq!(bytes, MODULE_V2.to_vec());
}

#[tokio::test]
async fn list_surfaces_pools_with_current_version_meta() {
    let _scoped = scoped();
    let router = app(state().await);
    create(router.clone(), "zzz-late").await;
    create(router.clone(), "aaa-first").await;
    let (status, v) = call(router, "GET", "/api/dag/binaries", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true);
    let pools = v["pools"].as_array().unwrap().clone();
    let names: Vec<&str> = pools.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["aaa-first", "zzz-late"], "sorted by name");
    assert_eq!(pools[0]["current"], 1);
    assert_eq!(pools[0]["current_version"]["sha256"], sha256_hex(MODULE));
    assert_eq!(pools[0]["current_version"]["version"], 1);
}

#[tokio::test]
async fn delete_removes_the_whole_pool() {
    let scoped_pair = scoped();
    let dir = scoped_pair.0;
    let router = app(state().await);
    create(router.clone(), "adder").await;

    let (status, v) = call(router.clone(), "DELETE", "/api/dag/binaries/adder", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["deleted"], "adder");
    assert!(!dir.path().join("adder").exists(), "pool dir must be gone");

    let (status, _v) = call(router, "GET", "/api/dag/binaries/adder", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
