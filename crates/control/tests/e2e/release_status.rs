//! Platform release status: disabled without a platform, merged view with one.
use crate::support::Harness;
use axum::{routing::get, Json, Router};
use reqwest::Method;
use serde_json::{json, Value};

#[tokio::test]
async fn release_status_without_platform_reports_disabled() {
    let h = Harness::new().await;
    let (status, body) = h.req(Method::GET, "/api/admin/release", None).await;
    assert_eq!(status, 200, "{body}");
    // RpcReply::ok passes the value through as the JSON body verbatim.
    assert_eq!(body, json!({"enabled": false}));
}

#[tokio::test]
async fn release_status_with_platform_merges_state_and_host() {
    let h = Harness::new().await;
    let state_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        state_dir.path().join("release-state.json"),
        json!({"phase":"steady"}).to_string(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/status", get(|| async { Json(json!({"healthy": true})) }));
    let host = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    h.state
        .lifecycle
        .platform
        .set(opencoder_core::fleet::release::PlatformConfig {
            release_id: "rel-e2e-1".into(),
            state_dir: state_dir.path().to_path_buf(),
            host_service: format!("http://{addr}"),
            resource_service: format!("http://{addr}"),
        })
        .unwrap();
    let (status, body): (reqwest::StatusCode, Value) =
        h.req(Method::GET, "/api/admin/release", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["instance_release"], "rel-e2e-1");
    assert_eq!(body["release"], json!({"phase":"steady"}));
    assert_eq!(body["host"], json!({"healthy":true}));
    assert_eq!(body["retiring"], false);
    assert_eq!(
        body["signal_protocol"],
        if cfg!(unix) { json!(1) } else { json!(0) }
    );
    host.abort();
}
