use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use base64::Engine;
use serde_json::{json, Value};
use tower::ServiceExt;

fn binary() -> Vec<u8> {
    let mut bytes = vec![0; 3 * 1024 * 1024];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[16] = 2;
    bytes[18..20].copy_from_slice(
        &if cfg!(target_arch = "aarch64") {
            183u16
        } else {
            62u16
        }
        .to_le_bytes(),
    );
    bytes[20] = 1;
    bytes[32] = 64;
    bytes[52] = 64;
    bytes[54] = 56;
    bytes[56] = 1;
    bytes[64] = 1;
    bytes
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    value: Value,
    token: Option<&str>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(value.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    (status, bytes)
}

#[tokio::test]
async fn resource_service_publishes_native_versions_to_its_exported_pool() {
    let temporary = tempfile::tempdir().unwrap();
    let workdir = temporary.path().join("work");
    let pool = temporary.path().join("export/binaries");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(
        workdir.join("opencoder.json"),
        json!({"dag":{"binary_dir":pool}}).to_string(),
    )
    .unwrap();
    let state = opencoder_control::new_state(workdir, temporary.path().join("state"), None)
        .await
        .unwrap();
    let token = "resource-service-test";
    let app = opencoder_control::release::resources::build_app(state, token.into());
    let health = call(&app, "GET", "/api/health", Value::Null, Some(token)).await;
    assert_eq!(health.0, StatusCode::OK);
    let health: Value = serde_json::from_slice(&health.1).unwrap();
    assert_eq!(health["role"], "resources");
    assert_eq!(
        health["build"],
        serde_json::to_value(opencoder_core::version::build_info()).unwrap()
    );
    let bytes = binary();
    let body = json!({"name":"tool","description":"native","binary_b64":base64::engine::general_purpose::STANDARD.encode(&bytes)});
    let unauthorized = call(&app, "POST", "/api/dag/binaries", body.clone(), None).await;
    assert_eq!(unauthorized.0, StatusCode::UNAUTHORIZED);
    assert!(!pool.join("tool").exists());
    let created = call(&app, "POST", "/api/dag/binaries", body.clone(), Some(token)).await;
    assert_eq!(
        created.0,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.1)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&created.1).unwrap()["version"],
        1
    );
    let conflict = call(&app, "POST", "/api/dag/binaries", body.clone(), Some(token)).await;
    assert_eq!(conflict.0, StatusCode::CONFLICT);
    let next = call(&app, "PUT", "/api/dag/binaries/tool", body, Some(token)).await;
    assert_eq!(next.0, StatusCode::OK);
    let rollback = call(
        &app,
        "POST",
        "/api/dag/binaries/tool/rollback",
        json!({"version":1}),
        Some(token),
    )
    .await;
    assert_eq!(rollback.0, StatusCode::OK);
    let downloaded = call(
        &app,
        "GET",
        "/api/dag/binaries/tool/versions/2/binary.bin",
        Value::Null,
        Some(token),
    )
    .await;
    assert_eq!(downloaded.0, StatusCode::OK);
    assert_eq!(downloaded.1, bytes);
    let metadata = opencoder_dag_binary::read_pool_meta(&pool, "tool").unwrap();
    assert_eq!(metadata.current, 1);
    assert_eq!(metadata.history.len(), 2);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let resource_service = format!("http://{}", listener.local_addr().unwrap());
    let owner = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let server_workdir = temporary.path().join("server-work");
    let unused_pool = temporary.path().join("server-only-pool");
    std::fs::create_dir_all(&server_workdir).unwrap();
    std::fs::write(
        server_workdir.join("opencoder.json"),
        json!({"dag":{"binary_dir":unused_pool}}).to_string(),
    )
    .unwrap();
    let server =
        opencoder_control::new_state(server_workdir, temporary.path().join("server-state"), None)
            .await
            .unwrap();
    server
        .lifecycle
        .platform
        .set(opencoder_core::fleet::release::PlatformConfig {
            release_id: "native-test".into(),
            state_dir: temporary.path().join("release-state"),
            host_service: "http://127.0.0.1:1".into(),
            resource_service,
        })
        .unwrap();
    let server = opencoder_control::build_app(server, Some(token.into()), false);
    let uploaded = call(
        &server,
        "POST",
        "/api/dag/binaries",
        json!({
            "name":"forwarded", "description":"published by the owner",
            "binary_b64":base64::engine::general_purpose::STANDARD.encode(&bytes),
        }),
        Some(token),
    )
    .await;
    assert_eq!(
        uploaded.0,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&uploaded.1)
    );
    assert!(pool.join("forwarded").is_dir());
    assert!(!unused_pool.exists());
    let response = server
        .oneshot(
            Request::builder()
                .uri("/api/dag/binaries/forwarded/versions/1/binary.bin")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"forwarded-v1.binary\""
    );
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap()
            .as_ref(),
        bytes.as_slice()
    );
    owner.abort();
}
