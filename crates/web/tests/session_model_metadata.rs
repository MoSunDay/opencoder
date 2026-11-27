use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::Router;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{SessionMeta, SessionModelOverrides, Store, TursoStore};
use tower::ServiceExt;

async fn app() -> (Router, Arc<opencoder_web::AppState>) {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let mock = MockChatClient::new().push_script(vec![LlmEvent::Completed {
        text: "done".into(),
        tool_calls: vec![],
        usage: None,
    }]);
    let client: Arc<dyn ChatStream> = Arc::new(mock);
    let state = Arc::new(opencoder_web::AppState {
        config_home: None,
        client_override: Some(client),
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store: store.clone(),
        workdir: tempfile::tempdir().unwrap().keep(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
    });
    let app = Router::new()
        .route("/api/sessions", post(opencoder_web::api::create_session))
        .route(
            "/api/sessions/:id/prompt",
            post(opencoder_web::api::post_prompt),
        )
        .with_state(state.clone());
    (app, state)
}

async fn post_json(
    app: Router,
    uri: &str,
    value: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn explicit_create_writes_empty_object_not_historical_null() {
    let (app, state) = app().await;
    let (status, body) = post_json(app, "/api/sessions", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["id"].as_str().unwrap();
    assert_eq!(
        state.store.get_session_model_overrides(id).await.unwrap(),
        Some(SessionModelOverrides::default())
    );

    state
        .store
        .create_session(&SessionMeta {
            id: "historical-null".into(),
            model: Some("legacy/model".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        state
            .store
            .get_session_model_overrides("historical-null")
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn implicit_pre_prompt_create_writes_empty_object() {
    let (app, state) = app().await;
    assert!(state.store.get_session("implicit").await.unwrap().is_none());
    let (status, body) = post_json(
        app,
        "/api/sessions/implicit/prompt",
        serde_json::json!({"prompt": "hello", "model": "mock/model"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(
        state
            .store
            .get_session_model_overrides("implicit")
            .await
            .unwrap(),
        Some(SessionModelOverrides::default())
    );
}
