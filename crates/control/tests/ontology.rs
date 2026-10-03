use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    value: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
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
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn authenticated_ontology_routes_and_resource_export_share_files_without_exporting_database()
{
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    let files = root.path().join("files");
    std::fs::create_dir(&work).unwrap();
    std::fs::write(
        work.join("opencoder.json"),
        json!({"ontology":{"files_dir":files,"nfs":{"port":0}}}).to_string(),
    )
    .unwrap();
    let state = opencoder_control::new_state(work, root.path().join("data"), None)
        .await
        .unwrap();
    let app = opencoder_control::build_app(state.clone(), Some("ontology-test".into()), false);
    assert_eq!(
        call(&app, "GET", "/api/ontology/environments", Value::Null, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let environments = call(
        &app,
        "GET",
        "/api/ontology/environments",
        Value::Null,
        Some("ontology-test"),
    )
    .await;
    assert_eq!(environments.0, StatusCode::OK);
    assert_eq!(environments.1["items"][0]["env_key"], "debug");
    assert!(root.path().join("data/ontology.db").is_file());
    assert!(!files.join("ontology.db").exists());
    let resources = opencoder_control::release::resources::build_app(state, "ontology-test".into());
    let started = call(
        &resources,
        "POST",
        "/api/ontology/nfs",
        json!({"enabled":true}),
        Some("ontology-test"),
    )
    .await;
    assert_eq!(started.0, StatusCode::OK, "{}", started.1);
    assert_eq!(started.1["status"]["running"], true);
    assert_eq!(started.1["status"]["read_only"], true);
    let port = started.1["status"]["port"].as_u64().unwrap();
    let again = call(
        &resources,
        "POST",
        "/api/ontology/nfs",
        json!({"enabled":true}),
        Some("ontology-test"),
    )
    .await;
    assert_eq!(again.1["status"]["port"], port);
    tokio::net::TcpStream::connect(("127.0.0.1", port as u16))
        .await
        .unwrap();
    let stopped = call(
        &resources,
        "POST",
        "/api/ontology/nfs",
        json!({"enabled":false}),
        Some("ontology-test"),
    )
    .await;
    assert_eq!(stopped.1["status"]["running"], false);
}
