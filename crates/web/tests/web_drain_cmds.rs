//! Integration tests for the web-parity drain commands (`SetApMode`,
//! `SetAnnotation`, `ResetPlanPhase`) and the plan-switch persistence in
//! `POST /agent`. Commands are queued via the public endpoints while a drain
//! is held mid-turn (mock `push_hang`), then applied when the run loop
//! returns — asserting the durable store state afterwards.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, SessionPatch, Store};
use serde_json::json;
use tokio::sync::Notify;
use tower::ServiceExt;

struct Ctx {
    app: axum::Router,
    store: Arc<dyn Store>,
    handles: opencoder_web::handle::HandleMap,
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
    });
    Ctx {
        app: opencoder_web::build_app(state, None, false),
        store,
        handles,
    }
}

async fn plain_app() -> Ctx {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = tempfile::tempdir().unwrap().keep();
    let handles = opencoder_web::handle::new_handle_map();
    let state = Arc::new(opencoder_web::AppState {
        store: store.clone(),
        workdir,
        handles: handles.clone(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        client_override: Some(Arc::new(MockChatClient::new().with_default(vec![
            LlmEvent::Completed {
                text: "t".into(),
                tool_calls: vec![],
                usage: None,
            },
        ])) as Arc<dyn ChatStream>),
    });
    Ctx {
        app: opencoder_web::build_app(state, None, false),
        store,
        handles,
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

/// `DrainCmd::ResetPlanPhase` zeroes (and persists) the plan-phase input
/// counter when delivered mid-drain — the TUI plan-switch parity path that
/// `POST /agent` forwards in its TOCTOU window.
#[tokio::test]
async fn reset_plan_phase_cmd_persists_zero_counter() {
    let notify = Arc::new(Notify::new());
    let ctx = hanging_app(notify.clone()).await;
    let id = create_session(&ctx).await;
    // Seed a nonzero counter so the reset is observable.
    ctx.store
        .update_session(
            &id,
            &SessionPatch {
                plan_input_count: Some(5),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    post_prompt(&ctx, &id, "hold this turn").await;
    assert!(
        opencoder_web::handle::send_cmd(
            &ctx.handles,
            &id,
            opencoder_web::cmd::DrainCmd::ResetPlanPhase
        )
        .await,
        "drain command must be delivered to the live handle"
    );
    notify.notify_one();
    wait_meta(&ctx.store, &id, |m| m.plan_input_count == 0, "counter = 0").await;
}

/// The common (not draining) plan-switch path persists `plan_input_count = 0`
/// so the next resume re-arms fresh plan-phase affordances.
#[tokio::test]
async fn agent_switch_to_plan_persists_zero_plan_input_count() {
    let ctx = plain_app().await;
    let id = create_session(&ctx).await;
    ctx.store
        .update_session(
            &id,
            &SessionPatch {
                plan_input_count: Some(3),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(
        post_json(
            &ctx,
            &format!("/api/sessions/{id}/agent"),
            json!({"value": "plan"}).to_string()
        )
        .await,
        StatusCode::OK
    );
    wait_meta(&ctx.store, &id, |m| m.plan_input_count == 0, "counter = 0").await;

    // Switching to a non-plan agent must NOT touch the counter.
    ctx.store
        .update_session(
            &id,
            &SessionPatch {
                plan_input_count: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        post_json(
            &ctx,
            &format!("/api/sessions/{id}/agent"),
            json!({"value": "act"}).to_string()
        )
        .await,
        StatusCode::OK
    );
    tokio::time::sleep(Duration::from_millis(150)).await;
    let meta = ctx.store.get_session(&id).await.unwrap().unwrap();
    assert_eq!(
        meta.plan_input_count, 2,
        "act switch must not reset the counter"
    );
}
