//! Bearer-auth middleware contract for protected server routes.

mod support;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use opencoder_store::{LibsqlStore, Store};
use support::authed_req;
use tower::ServiceExt;

const TOKEN: &str = "sekret-token-123";

async fn app() -> axum::Router {
    app_with_web(true).await
}

async fn app_with_web(web: bool) -> axum::Router {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
        client_override: None,
    });
    opencoder_web::build_app(state, Some(TOKEN.into()), web)
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.clone().oneshot(req).await.expect("router must answer");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}))
    };
    (status, body)
}

#[tokio::test]
async fn missing_authorization_is_401() {
    let (status, body) = send(
        &app().await,
        Request::builder()
            .uri("/api/health")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "invalid bearer token");

    let (status, _) = send(
        &app().await,
        Request::builder()
            .uri("/api/health")
            .header("x-sig", "retired-signature")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_or_malformed_authorization_is_401() {
    let app = app().await;
    for value in [
        "Bearer wrong",
        "Bearer Sekret-token-123",
        "Basic sekret-token-123",
        "Bearer ",
    ] {
        let (status, _) = send(
            &app,
            Request::builder()
                .uri("/api/health")
                .header(header::AUTHORIZATION, value)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "header={value:?}");
    }
}

#[tokio::test]
async fn valid_bearer_token_is_200() {
    let app = app().await;
    for _ in 0..2 {
        let (status, _) = send(&app, authed_req("GET", "/api/health", TOKEN, None)).await;
        assert_eq!(status, StatusCode::OK);
    }

    let (status, _) = send(
        &app,
        Request::builder()
            .uri("/api/health")
            .header(header::AUTHORIZATION, format!("bearer  {TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn bearer_auth_does_not_consume_or_transform_json_body() {
    let body = serde_json::json!({ "name": "n1" });
    let (status, _) = send(
        &app().await,
        authed_req("POST", "/api/nodes/register", TOKEN, Some(body.to_string())),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn time_endpoint_is_unauthenticated() {
    let (status, body) = send(
        &app().await,
        Request::builder()
            .uri("/api/time")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["server_time_ms"].is_number());
}

#[tokio::test]
async fn shell_paths_are_unauthenticated_but_protected_api_is_not() {
    let app = app_with_web(true).await;
    let shell = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(shell.status(), StatusCode::OK);

    let api = app
        .oneshot(
            Request::builder()
                .uri("/api/nodes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(api.status(), StatusCode::UNAUTHORIZED);
}
