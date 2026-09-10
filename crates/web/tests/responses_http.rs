#[path = "../../llm/tests/responses/support.rs"]
mod wire;

use axum::{body::Body, http::Request};
use opencoder_store::{LibsqlStore, Store};
use serde_json::json;
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tower::ServiceExt;
use wire::*;

#[tokio::test]
async fn web_prompt_builds_responses_client_executes_edit_and_persists_sse() {
    let dir = tempfile::tempdir().unwrap();
    let _home = opencoder_core::scoped_config_home(dir.path().to_path_buf());
    let server = serve(vec![
        Reply::events(vec![completed(vec![
            reasoning(),
            call(
                "edit1",
                "edit",
                json!({"path":"a.txt","old_string":"broken","new_string":"fixed"}),
            ),
        ])]),
        Reply::events(vec![completed(vec![answer("Task complete")])]),
        Reply::events(vec![completed(vec![answer("Fix file")])]),
    ])
    .await;
    std::fs::create_dir_all(dir.path().join(".opencoder")).unwrap();
    std::fs::write(dir.path().join("a.txt"), "broken").unwrap();
    std::fs::write(
        dir.path().join(".opencoder/config.json"),
        serde_json::to_vec(&config(&server.url)).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.path().join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let handles = opencoder_web::handle::new_handle_map();
    let app = opencoder_web::build_app(
        Arc::new(opencoder_web::AppState {
            brain: opencoder_web::api_brain::mock_brain(store.clone()),
            store: store.clone(),
            workdir: dir.path().to_path_buf(),
            handles: handles.clone(),
            nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
            controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
            team: opencoder_web::team_state::mock(),
            project: opencoder_web::ProjectService::new(),
            client_override: None,
        }),
        None,
        false,
    );
    let request = |uri: String, body: serde_json::Value| {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let response = app
        .clone()
        .oneshot(request("/api/sessions".into(), json!({})))
        .await
        .unwrap();
    assert!(response.status().is_success());
    let created: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap();
    let id = created["id"].as_str().unwrap();
    let response = app
        .oneshot(request(
            format!("/api/sessions/{id}/prompt"),
            json!({"prompt":"Fix a.txt"}),
        ))
        .await
        .unwrap();
    assert!(response.status().is_success());
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let finished = handles
                .lock()
                .await
                .get(id)
                .is_some_and(|h| !h.draining.load(Ordering::SeqCst));
            if finished
                && store
                    .load_messages(id)
                    .await
                    .unwrap()
                    .iter()
                    .any(|m| m.text() == "Task complete")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("web coding task did not settle");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "fixed"
    );
    let messages = store.load_messages(id).await.unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.provider_state.is_some())
            .count(),
        2
    );
    let events = store.events_after(id, 0).await.unwrap();
    assert!(events
        .iter()
        .any(|e| e.sse_kind.as_deref() == Some("tool_end")));
    assert!(server
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|(h, _)| h.starts_with("POST /responses ")));
}
