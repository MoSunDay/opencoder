//! Control-plane mounting of the `/api/dag/wasm` pool: the shared web
//! handlers plus the `api_dag_wasm_nfs::configured_dag_wasm` scope
//! middleware must resolve the pool root per workdir (config
//! `dag.wasm_dir`, else the data-dir default) so two servers on one
//! host never leak pools into each other. Harness mirrors
//! `resource_root.rs`.

use axum::{body::Body, http::Request, Router};
use base64::Engine;
use serde_json::{json, Value};
use tower::ServiceExt;

/// Minimal valid module: 8-byte wasm header + payload (the pool layer
/// only checks the header, never runs the module).
fn module(payload: &str) -> Vec<u8> {
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    bytes.extend_from_slice(payload.as_bytes());
    bytes
}

async fn api(app: &Router, method: &str, path: &str, value: Value) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    assert!(
        status.is_success(),
        "{status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

/// Binary GET (wasm.bin download): status-checked raw bytes.
async fn download(app: &Router, path: &str) -> Vec<u8> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    assert!(status.is_success(), "{status}");
    bytes.to_vec()
}

/// Publish the shared pool name `tool` with a per-server payload so a
/// cross-server leak would surface as mismatched bytes.
async fn publish(app: &Router, payload: &str) {
    api(
        app,
        "POST",
        "/api/dag/wasm",
        json!({
            "name": "tool",
            "description": "d",
            "wasm_b64": base64::engine::general_purpose::STANDARD.encode(module(payload)),
        }),
    )
    .await;
}

#[tokio::test]
async fn wasm_pool_publication_uses_configured_root_without_cross_server_leaks() {
    let dir = tempfile::tempdir().unwrap();
    let mut apps = Vec::new();
    for name in ["first", "second"] {
        let workdir = dir.path().join(name);
        let root = workdir.join("pool");
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::write(
            workdir.join("opencoder.json"),
            json!({"dag":{"wasm_dir":root}}).to_string(),
        )
        .unwrap();
        let state = opencoder_control::new_state(workdir.clone(), workdir.join("state"), None)
            .await
            .unwrap();
        apps.push((opencoder_control::build_app(state, None, false), root));
    }
    tokio::join!(publish(&apps[0].0, "first"), publish(&apps[1].0, "second"));
    for (index, other) in [(0, 1), (1, 0)] {
        let (app, root) = &apps[index];
        let own = module(if index == 0 { "first" } else { "second" });
        let theirs = module(if other == 0 { "first" } else { "second" });
        assert!(root.join("tool").join("meta.json").is_file());
        assert_eq!(
            std::fs::read(root.join("tool").join("v1").join("wasm.bin")).unwrap(),
            own
        );
        // The other server's root holds only its own copy, never ours.
        let other_root = &apps[other].1;
        assert_eq!(
            std::fs::read(other_root.join("tool").join("v1").join("wasm.bin")).unwrap(),
            theirs
        );
        let listing = api(app, "GET", "/api/dag/wasm", Value::Null).await;
        let names: Vec<&str> = listing["pools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["tool"]);
        let meta = api(app, "GET", "/api/dag/wasm/tool", Value::Null).await;
        assert_eq!(meta["current"], 1);
        assert_eq!(meta["name"], "tool");
        let bytes = download(app, "/api/dag/wasm/tool/versions/1/wasm.bin").await;
        assert_eq!(bytes, own);
    }
}

#[tokio::test]
async fn pool_scope_defaults_to_workdir_data_dir_when_unconfigured() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    let state = opencoder_control::new_state(workdir.clone(), workdir.join("state"), None)
        .await
        .unwrap();
    let app = opencoder_control::build_app(state, None, false);
    publish(&app, "default").await;
    // No `dag.wasm_dir` config: the pool lands under the per-workdir
    // data-dir default (`<data_dir>/dag/wasm`) — pins that contract.
    let default_root = opencoder_core::data_dir_for(&workdir)
        .join("dag")
        .join("wasm");
    assert!(default_root.join("tool").join("meta.json").is_file());
    assert!(default_root
        .join("tool")
        .join("v1")
        .join("wasm.bin")
        .is_file());
    // And the read API serves it from the same resolved root.
    let meta = api(&app, "GET", "/api/dag/wasm/tool", Value::Null).await;
    assert_eq!(meta["current"], 1);
}

#[test]
fn role_gate_wasm_pool_is_read_only_for_users() {
    use axum::http::Method;
    use opencoder_core::identity::Role;
    let allowed = |method: &str, path: &str| {
        opencoder_control::role_gate::allowed(
            Role::User,
            &Method::from_bytes(method.as_bytes()).unwrap(),
            path,
        )
    };
    assert!(allowed("GET", "/api/dag/wasm"));
    assert!(allowed("GET", "/api/dag/wasm/tool"));
    assert!(allowed("GET", "/api/dag/wasm/tool/versions/1/wasm.bin"));
    assert!(!allowed("POST", "/api/dag/wasm"));
    assert!(!allowed("PUT", "/api/dag/wasm/tool"));
    assert!(!allowed("DELETE", "/api/dag/wasm/tool"));
    assert!(!allowed("POST", "/api/dag/wasm/tool/rollback"));
}
