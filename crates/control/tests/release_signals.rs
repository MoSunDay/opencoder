use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use opencoder_control::release::signals;
use opencoder_core::fleet::release::PlatformConfig;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn signals_forward_authenticated_actions_and_propagate_rejection_without_retiring() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(dir.path().join("home"));
    let state =
        opencoder_control::new_state(dir.path().join("work"), dir.path().join("control"), None)
            .await
            .unwrap();
    assert!(signals::request(&state, "deploy")
        .await
        .unwrap_err()
        .to_string()
        .contains("not configured"));
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    async fn receive(
        State(requests): State<Arc<Mutex<Vec<Value>>>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        assert_eq!(headers["authorization"], "Bearer signal-fixture");
        assert_eq!(body["release_id"], "r1");
        requests.lock().unwrap().push(body.clone());
        if body["action"] == "rollback" {
            (
                StatusCode::CONFLICT,
                Json(json!({"error":"no compatible previous release"})),
            )
        } else {
            (StatusCode::OK, Json(json!({"accepted":true})))
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host_service = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/deployment-signal", post(receive))
        .with_state(requests.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    state
        .lifecycle
        .platform
        .set(PlatformConfig {
            release_id: "r1".into(),
            state_dir: dir.path().into(),
            host_service,
            resource_service: "http://127.0.0.1:1".into(),
        })
        .unwrap();
    state
        .lifecycle
        .credential
        .set("signal-fixture".into())
        .unwrap();
    assert_eq!(
        signals::request(&state, "deploy").await.unwrap(),
        json!({"accepted":true})
    );
    assert!(signals::request(&state, "rollback")
        .await
        .unwrap_err()
        .to_string()
        .contains("no compatible previous release"));
    assert!(!state
        .lifecycle
        .retiring
        .load(std::sync::atomic::Ordering::SeqCst));
    assert!(signals::request(&state, "stop").await.is_err());
    state.lifecycle.retire();
    assert!(signals::request(&state, "deploy")
        .await
        .unwrap_err()
        .to_string()
        .contains("retiring"));
    assert_eq!(requests.lock().unwrap().len(), 2);
    task.abort();
}
