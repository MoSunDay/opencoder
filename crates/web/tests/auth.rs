//! Auth middleware contract: every route (/ and /api/*) requires a valid bearer
//! token via the `Authorization: Bearer` header OR a `?token=` query (so the
//! browser EventSource API, which cannot set headers, still works). Mismatched
//! or missing tokens yield 401.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use opencoder_store::{LibsqlStore, Store};

const TOKEN: &str = "sekret-token-123";

async fn app() -> axum::Router {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        client_override: None,
    });
    opencoder_web::build_app(state, Some(TOKEN.into()))
}

#[tokio::test]
async fn health_without_token_is_401() {
    let app = app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn health_with_wrong_bearer_is_401() {
    let app = app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .header("authorization", "Bearer nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn health_with_correct_bearer_is_200() {
    let app = app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_with_correct_query_token_is_200() {
    let app = app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/health?token={TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn index_html_protected_by_query_token() {
    let app = app().await;
    // no token -> 401
    let r1 = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::UNAUTHORIZED);
    // ?token= -> 200 HTML
    let r2 = app
        .oneshot(
            Request::builder()
                .uri(format!("/?token={TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
}

#[tokio::test]
async fn sessions_list_requires_token() {
    let app = app().await;
    // wrong token
    let r1 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .header("authorization", "Bearer wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::UNAUTHORIZED);
    // correct token
    let r2 = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
}
