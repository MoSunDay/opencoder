//! Integration tests for post-drain LLM title generation over HTTP: a
//! successful drain's follow-up small-model call persists the generated
//! title, and a session that already carries a title skips the extra LLM
//! round entirely. Driven through the real router with a `MockChatClient`.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, SessionPatch, Store};
use serde_json::json;
use tower::ServiceExt;

struct Ctx {
    app: axum::Router,
    store: Arc<dyn Store>,
    handles: opencoder_web::handle::HandleMap,
    workdir: std::path::PathBuf,
    mock: Arc<MockChatClient>,
}

/// App with a scripted mock and a pinned-off autopilot (a developer's global
/// ap.json must not append a review round to this two-call sequence). The
/// project config pins model + small_model so the title request's model is
/// assertable.
async fn app(mock: MockChatClient) -> Ctx {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("config.json"),
        json!({"model": "a/big", "small_model": "a/mini"}).to_string(),
    )
    .unwrap();
    let mock = Arc::new(mock);
    let handles = opencoder_web::handle::new_handle_map();
    let state = Arc::new(opencoder_web::AppState {
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store: store.clone(),
        workdir: workdir.clone(),
        handles: handles.clone(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
        client_override: Some(mock.clone() as Arc<dyn ChatStream>),
    });
    Ctx {
        app: opencoder_web::build_app(state, None, false),
        store,
        handles,
        workdir,
        mock,
    }
}

fn text_round(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
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
    assert_eq!(resp.status(), StatusCode::OK);
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
    assert_eq!(resp.status(), StatusCode::OK, "prompt must be admitted");
}

/// Poll the persisted transcript until it contains `needle` (bounded).
async fn wait_for_transcript(store: &Arc<dyn Store>, id: &str, needle: &str) {
    for _ in 0..300 {
        let msgs = store.load_messages(id).await.unwrap();
        let all = msgs.iter().map(|m| m.estimate_chars()).collect::<String>();
        if all.contains(needle) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("transcript never contained {needle:?}");
}

/// Wait until the session's drain has fully exited (draining flag false).
/// `maybe_generate_title` is awaited BEFORE the flag flips, so once this
/// returns, title generation has either run or been skipped.
async fn wait_drain_exited(ctx: &Ctx, id: &str) {
    for _ in 0..300 {
        let settled = {
            let map = ctx.handles.lock().await;
            map.get(id)
                .map(|h| !h.draining.load(Ordering::SeqCst))
                .unwrap_or(false)
        };
        if settled {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("drain never exited");
}

async fn persisted_title(store: &Arc<dyn Store>, id: &str) -> Option<String> {
    store.get_session(id).await.unwrap().and_then(|m| m.title)
}

/// A successful drain makes one assistant round, then a bounded small-model
/// call whose Completed text becomes the persisted session title.
#[tokio::test]
async fn successful_drain_persists_generated_title() {
    let mock = MockChatClient::new()
        .push_script(vec![text_round("the work is done")])
        .push_script(vec![text_round("mock title")]);
    let ctx = app(mock).await;
    let id = create_session(&ctx).await;

    post_prompt(&ctx, &id, "summarize the repo").await;
    wait_for_transcript(&ctx.store, &id, "the work is done").await;
    wait_drain_exited(&ctx, &id).await;

    // Bounded poll: the title write happens on the drain task.
    let mut title = None;
    for _ in 0..100 {
        title = persisted_title(&ctx.store, &id).await;
        if title.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(title.as_deref(), Some("mock title"));

    // The title round must target the small model, not the primary.
    let reqs = ctx.mock.requests();
    assert!(reqs.len() >= 2, "expected a run round + a title round");
    assert_eq!(reqs.last().unwrap().model, "mini");
    assert_eq!(reqs[0].model, "big");
    // The drain pinned the project config: primary id must be the default.
    let cfg = opencoder_core::Config::load(&ctx.workdir).unwrap();
    // small_model_or_primary strips the provider prefix ("a/mini" -> "mini").
    assert_eq!(cfg.small_model_or_primary(), "mini");
}

/// A session that already has a title skips generation entirely: the
/// follow-up round would overwrite nothing, and the default script (which
/// returns a distinctive "must not appear" title) proves no extra LLM call
/// lands.
#[tokio::test]
async fn existing_title_skips_generation() {
    let mock = MockChatClient::new()
        .push_script(vec![text_round("the work is done")])
        .with_default(vec![text_round("TITLE-SHOULD-NOT-APPEAR")]);
    let ctx = app(mock).await;
    let id = create_session(&ctx).await;
    ctx.store
        .update_session(
            &id,
            &SessionPatch {
                title: Some("Existing Title".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    post_prompt(&ctx, &id, "summarize the repo").await;
    wait_for_transcript(&ctx.store, &id, "the work is done").await;
    wait_drain_exited(&ctx, &id).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        persisted_title(&ctx.store, &id).await.as_deref(),
        Some("Existing Title"),
        "an existing title must never be overwritten"
    );
}
