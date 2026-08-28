//! Integration tests for the web-parity drain commands (`SetApMode`,
//! `SetAnnotation`). Commands are queued via the public endpoints while a
//! drain is held mid-turn (mock `push_hang`), then applied when the run loop
//! returns — asserting the durable store state afterwards.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use serde_json::json;
use tokio::sync::Notify;
use tower::ServiceExt;

struct Ctx {
    app: axum::Router,
    store: Arc<dyn Store>,
}

async fn hanging_app(notify: Arc<Notify>) -> Ctx {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
    let mock = MockChatClient::new()
        .push_hang(notify)
        .with_default(vec![LlmEvent::Completed {
            text: "done".into(),
            tool_calls: vec![],
            usage: None,
        }]);
    let handles = opencoder_web::handle::new_handle_map();
    let state = Arc::new(opencoder_web::AppState {
        store: store.clone(),
        workdir,
        handles: handles.clone(),
        client_override: Some(Arc::new(mock) as Arc<dyn ChatStream>),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
    });
    Ctx {
        app: opencoder_web::build_app(state, None, false),
        store,
    }
}

async fn create_session(ctx: &Ctx) -> String {
    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 4096).await.unwrap())
            .unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn post_prompt(ctx: &Ctx, id: &str, prompt: &str) {
    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{id}/prompt"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"prompt": prompt}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "admit failed");
}

async fn post_json(ctx: &Ctx, uri: &str, body: String) -> StatusCode {
    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    resp.status()
}

/// Wait until the session row reports the given autopilot/requirement state.
async fn wait_meta<F: Fn(&opencoder_store::SessionMeta) -> bool>(
    store: &Arc<dyn Store>,
    id: &str,
    pred: F,
    what: &str,
) {
    for _ in 0..300 {
        if let Ok(Some(meta)) = store.get_session(id).await {
            if pred(&meta) {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("session meta never reached {what}");
}

/// Endpoint-posted autopilot/annotation updates are forwarded as drain
/// commands while a drain runs, and applied (persisted) once the turn ends.
#[tokio::test]
async fn autopilot_and_annotation_cmds_apply_to_live_drain() {
    let notify = Arc::new(Notify::new());
    let ctx = hanging_app(notify.clone()).await;
    let id = create_session(&ctx).await;

    post_prompt(&ctx, &id, "hold this turn").await;
    // Drain is now stuck in its first LLM call: the endpoint sees
    // draining=true and forwards DrainCmds instead of relying on resume.
    assert_eq!(
        post_json(
            &ctx,
            &format!("/api/sessions/{id}/autopilot"),
            json!({"mode": "ap"}).to_string()
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        post_json(
            &ctx,
            &format!("/api/sessions/{id}/annotation"),
            json!({"text": "prefer sqlite"}).to_string()
        )
        .await,
        StatusCode::OK
    );

    // Release the hang: the run returns, `process_drain_cmds` applies both
    // commands, each persisting its column.
    notify.notify_one();
    wait_meta(
        &ctx.store,
        &id,
        |m| m.autopilot_mode.as_deref() == Some("ap"),
        "autopilot_mode = ap",
    )
    .await;
    wait_meta(
        &ctx.store,
        &id,
        |m| m.requirement.as_deref() == Some("prefer sqlite"),
        "requirement = prefer sqlite",
    )
    .await;
}
