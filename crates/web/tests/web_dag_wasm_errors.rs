//! `/api/dag/wasm` error-contract tests: 400 for bad names / bodies /
//! modules, 404 for unknown pools and versions. Harness in
//! `support/dag_wasm.rs` (same override-scoped root).

mod support;

use axum::http::StatusCode;

use support::dag_wasm::{app, b64, call, call_raw, create, scoped, state, MODULE};

#[tokio::test]
async fn rejects_invalid_names_bodies_and_modules() {
    let _scoped = scoped();
    let router = app(state().await);

    // Traversal and charset violations in the name.
    for name in ["../x", "a b"] {
        let (status, v) = call(
            router.clone(),
            "POST",
            "/api/dag/wasm",
            Some(serde_json::json!({ "name": name, "wasm_b64": b64(MODULE) })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "name `{name}`: {v}");
    }

    // Bad base64.
    let (status, v) = call(
        router.clone(),
        "POST",
        "/api/dag/wasm",
        Some(serde_json::json!({ "name": "adder", "wasm_b64": "!!!not-b64!!!" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap_or("").contains("base64"));

    // Wrong magic (ELF, not \0asm).
    let (status, _v) = call(
        router.clone(),
        "POST",
        "/api/dag/wasm",
        Some(serde_json::json!({ "name": "adder", "wasm_b64": b64(b"ELF-not-wasm") })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Empty module (empty wasm_b64 decodes to 0 bytes; the 32 MiB cap
    // branch is deliberately not exercised by allocating a huge body).
    let (status, _v) = call(
        router.clone(),
        "POST",
        "/api/dag/wasm",
        Some(serde_json::json!({ "name": "adder", "wasm_b64": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    create(router.clone(), "adder").await;
    // Non-numeric version in the download path.
    let (status, _v, _b) = call_raw(
        router.clone(),
        "GET",
        "/api/dag/wasm/adder/versions/x/wasm.bin",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Rollback to a version outside the history maps InvalidInput ⇒ 400.
    let (status, _v) = call(
        router,
        "POST",
        "/api/dag/wasm/adder/rollback",
        Some(serde_json::json!({ "version": 99 })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unknown_pool_and_version_are_404() {
    let _scoped = scoped();
    let router = app(state().await);
    create(router.clone(), "adder").await;

    let (status, _v) = call(router.clone(), "GET", "/api/dag/wasm/ghost", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _v) = call(
        router.clone(),
        "PUT",
        "/api/dag/wasm/ghost",
        Some(serde_json::json!({ "description": "d", "wasm_b64": b64(MODULE) })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _v) = call(router.clone(), "DELETE", "/api/dag/wasm/ghost", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _v) = call(
        router.clone(),
        "POST",
        "/api/dag/wasm/ghost/rollback",
        Some(serde_json::json!({ "version": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Existing pool, missing version.
    let (status, _v, _b) = call_raw(
        router.clone(),
        "GET",
        "/api/dag/wasm/adder/versions/99/wasm.bin",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
