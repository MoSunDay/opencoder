//! Closed-loop integration tests for the web `question` bridge: a plan
//! agent turn whose LLM round returns a `question` tool call blocks mid-drain until
//! the HTTP endpoints answer/skip it, then the follow-up LLM round completes
//! the turn. Driven through the real router with a `MockChatClient` (no
//! network), tempdir workdir, in-memory store.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use serde_json::json;
use tower::ServiceExt;

/// Build the app around a mock whose scripts are queued FIFO up front:
/// round 1 asks the question, round 2 is the post-answer follow-up, and the
/// default covers title generation / any later call so the queue never
/// starves.
async fn app(mock: MockChatClient) -> (axum::Router, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = tempfile::tempdir().unwrap().keep();
    // Pin autopilot off via the project domain file: a developer's global
    // ap.json must not append a review turn to this scripted sequence.
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
    let state = Arc::new(opencoder_web::AppState {
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store: store.clone(),
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
        client_override: Some(Arc::new(mock) as Arc<dyn ChatStream>),
    });
    (opencoder_web::build_app(state, None, false), store)
}

fn question_round() -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: "call_q1".into(),
            name: "question".into(),
            input: json!({"question": "which db?", "options": ["pg", "mysql"]}),
        }],
        usage: None,
    }
}

fn text_round(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Poll GET /questions until at least one question is open (bounded).
async fn wait_for_question(app: &axum::Router, id: &str) -> serde_json::Value {
    for _ in 0..200 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/sessions/{id}/questions"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1 << 20)
                .await
                .unwrap(),
        )
        .unwrap();
        if body["questions"].as_array().is_some_and(|q| !q.is_empty()) {
            return body["questions"][0].clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("question never became waiting within timeout");
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

async fn create_plan_session(app: &axum::Router) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"agent":"plan"}"#))
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

async fn post_plan_prompt(app: &axum::Router, id: &str, prompt: &str) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{id}/prompt"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"agent":"plan","prompt":prompt}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "prompt must be admitted");
}

async fn post(app: &axum::Router, uri: &str, body: String) -> (StatusCode, serde_json::Value) {
    let resp = app
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
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, body)
}

/// The full closed loop: question → poll → answer → follow-up round → the
/// answered text lands in the persisted transcript.
#[tokio::test]
async fn answer_flows_into_tool_result_and_completes_the_turn() {
    let mock = MockChatClient::new()
        .push_script(vec![question_round()])
        .push_script(vec![text_round("the answer will use pg")])
        .with_default(vec![text_round("t")]);
    let (app, store) = app(mock).await;
    let id = create_plan_session(&app).await;
    post_plan_prompt(&app, &id, "explore something ambiguous").await;

    let q = wait_for_question(&app, &id).await;
    assert_eq!(q["id"].as_str(), Some("call_q1"));
    assert_eq!(q["question"].as_str(), Some("which db?"));
    assert_eq!(
        q["options"].as_array().unwrap().len(),
        2,
        "options must be surfaced for the frontend"
    );

    let (status, body) = post(
        &app,
        &format!("/api/sessions/{id}/questions/call_q1/answer"),
        json!({"answer": "pg"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(true));

    // The answered value is the tool result; the follow-up round's text is
    // the final assistant reply.
    wait_for_transcript(&store, &id, "pg").await;
    wait_for_transcript(&store, &id, "the answer will use pg").await;
}

/// Skipping resolves the blocked tool to the fixed SKIPPED_REPLY.
#[tokio::test]
async fn skip_resolves_tool_to_skipped_reply() {
    let mock = MockChatClient::new()
        .push_script(vec![question_round()])
        .with_default(vec![text_round("ok, proceeding")]);
    let (app, store) = app(mock).await;
    let id = create_plan_session(&app).await;
    post_plan_prompt(&app, &id, "explore something ambiguous").await;

    wait_for_question(&app, &id).await;
    let (status, body) = post(
        &app,
        &format!("/api/sessions/{id}/questions/call_q1/skip"),
        String::new(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(true));
    wait_for_transcript(&store, &id, "User skipped").await;
}

/// Unknown call id → 404 on both answer and skip; empty answer → 400.
#[tokio::test]
async fn unknown_call_id_and_empty_answer_are_rejected() {
    let mock = MockChatClient::new().with_default(vec![text_round("t")]);
    let (app, _store) = app(mock).await;
    let id = create_plan_session(&app).await;

    let (status, body) = post(
        &app,
        &format!("/api/sessions/{id}/questions/nope/answer"),
        json!({"answer": "pg"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "answer: {body}");
    assert_eq!(body["ok"].as_bool(), Some(false));

    let (status, _body) = post(
        &app,
        &format!("/api/sessions/{id}/questions/nope/skip"),
        String::new(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // No question is waiting: an empty answer is a 400 even for a well-formed
    // call id (checked before the waiting lookup).
    let (status, body) = post(
        &app,
        &format!("/api/sessions/{id}/questions/call_q1/answer"),
        json!({"answer": "   "}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "empty answer: {body}");
}

/// Listing with nothing waiting is 200 + empty array (polling contract).
#[tokio::test]
async fn list_questions_empty_is_200_with_array() {
    let mock = MockChatClient::new().with_default(vec![text_round("t")]);
    let (app, _store) = app(mock).await;
    let id = create_plan_session(&app).await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{id}/questions"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 4096).await.unwrap())
            .unwrap();
    assert_eq!(body["questions"].as_array().map(Vec::len), Some(0));
}

/// When the LAST SSE subscriber disconnects while a question is waiting, the
/// handle's questions are abandoned — the blocked tool resolves to
/// SKIPPED_REPLY and the turn completes instead of hanging forever.
#[tokio::test]
async fn last_subscriber_disconnect_abandons_waiting_question() {
    use futures::StreamExt;

    let mock = MockChatClient::new()
        .push_script(vec![question_round()])
        .with_default(vec![text_round("proceeding alone")]);
    let (app, store) = app(mock).await;
    let id = create_plan_session(&app).await;

    // Subscribe BEFORE prompting: this request creates the handle and holds
    // the only subscriber slot.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{id}/events"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut stream = resp.into_body().into_data_stream();

    post_plan_prompt(&app, &id, "explore something ambiguous").await;
    let q = wait_for_question(&app, &id).await;
    assert_eq!(q["id"].as_str(), Some("call_q1"));

    // Drain a couple of SSE frames so the subscription is live, then DROP
    // the stream: the drop guard releases the last subscriber slot.
    let _ = tokio::time::timeout(Duration::from_millis(300), stream.next()).await;
    drop(stream);

    // The abandoned question must resolve to SKIPPED_REPLY and the turn must
    // complete (final text from the follow-up round lands in the store).
    wait_for_transcript(&store, &id, "User skipped").await;
    wait_for_transcript(&store, &id, "proceeding alone").await;
}
