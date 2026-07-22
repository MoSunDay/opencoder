//! End-to-end client ↔ server test. A real `opencoder server` router (with an
//! injected `MockChatClient` so no real LLM is hit) is bound to an ephemeral
//! TCP port; a real `opencoder_client::Remote` drives it over HTTP + SSE. We
//! assert the client's echoed event sequence matches the events the server
//! persisted — the core "client is a thin shell over the server" contract.
//!
//! Also covers auth over the wire: a correct token reaches the API, a wrong
//! token is rejected with 401.

use std::sync::Arc;
use std::time::Duration;

use opencoder_client::Remote;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

const TOKEN: &str = "e2e-bearer-token";

/// Build a server AppState whose drain uses a deterministic mock LLM that emits
/// one text delta then completes.
async fn state_with_mock() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta("hello from server".into()),
        LlmEvent::Completed {
            text: "hello from server".into(),
            tool_calls: vec![],
            usage: None,
        },
    ]));
    Arc::new(opencoder_web::AppState {
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        client_override: Some(mock),
    })
}

/// Spawn a server on an ephemeral port and return its base URL + the shared
/// AppState (so the test can inspect persisted records).
async fn spawn_server(state: Arc<opencoder_web::AppState>) -> String {
    let app = opencoder_web::build_app(state, Some(TOKEN.into()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn health_with_correct_token_succeeds_wrong_token_fails() {
    let state = state_with_mock().await;
    let base = spawn_server(state).await;

    let good = Remote::new(&base, TOKEN).unwrap();
    assert!(good.health().await.unwrap());

    let bad = Remote::new(&base, "wrong-token").unwrap();
    assert!(!bad.health().await.unwrap());
}

#[tokio::test]
async fn client_echo_matches_server_persisted_events() {
    let state = state_with_mock().await;
    let base = spawn_server(state.clone()).await;
    let remote = Remote::new(&base, TOKEN).unwrap();

    // 1. create a session
    let id = remote.create_session(None, None).await.unwrap();
    assert!(!id.is_empty());

    // 2. snapshot the event cursor
    let after = remote.last_event_seq(&id).await.unwrap();

    // 3. post the prompt
    let seq = remote
        .post_prompt(&id, "ping", None, None, None)
        .await
        .unwrap();
    assert!(seq > 0);

    // 4. stream events from the snapshot; collect kinds + echoed text
    let mut rx = remote.events(&id, after).unwrap();
    let mut kinds = Vec::new();
    let mut text = String::new();
    while let Some(evt) = rx.recv().await {
        if evt.kind == "text_delta" {
            if let Some(t) = evt.data.get("text").and_then(|v| v.as_str()) {
                text.push_str(t);
            }
        }
        kinds.push(evt.kind.clone());
        if evt.kind == "done" {
            break;
        }
        if evt.kind == "error" {
            panic!("server reported error: {:?}", evt.data);
        }
    }

    // The echo must include the streamed text and terminate with done.
    assert!(
        text.contains("hello from server"),
        "echoed text was {text:?}"
    );
    assert_eq!(kinds.last(), Some(&"done".to_string()), "kinds = {kinds:?}");

    // 5. the server persisted the SAME sequence (poll: the done persist is a
    //    spawned fire-and-forget task that may lag the live broadcast).
    let persisted_kinds = {
        let mut last = Vec::new();
        for _ in 0..80 {
            let recs = state.store.events_after(&id, after).await.unwrap();
            last = recs
                .iter()
                .map(|r| r.sse_kind.clone().unwrap_or_default())
                .collect::<Vec<_>>();
            if last.last() == Some(&"done".to_string()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        last
    };
    assert_eq!(
        persisted_kinds, kinds,
        "server persisted kinds must equal client-received kinds"
    );
}

#[tokio::test]
async fn client_get_messages_returns_transcript() {
    let state = state_with_mock().await;
    let base = spawn_server(state.clone()).await;
    let remote = Remote::new(&base, TOKEN).unwrap();

    let id = remote.create_session(None, None).await.unwrap();
    let after = remote.last_event_seq(&id).await.unwrap();
    let _ = remote
        .post_prompt(&id, "hello", None, None, None)
        .await
        .unwrap();

    // drain to completion
    let mut rx = remote.events(&id, after).unwrap();
    while let Some(evt) = rx.recv().await {
        if evt.kind == "done" {
            break;
        }
    }

    // transcript now contains the assistant text
    let mut got = Vec::new();
    for _ in 0..80 {
        got = remote.get_messages(&id).await.unwrap();
        if got.iter().any(|m| {
            m.role == opencoder_core::Role::Assistant
                && m.blocks
                    .iter()
                    .any(|b| b.as_text() == Some("hello from server"))
        }) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        got.iter().any(|m| m
            .blocks
            .iter()
            .any(|b| b.as_text() == Some("hello from server"))),
        "assistant text missing from transcript: {got:?}"
    );
}

#[tokio::test]
async fn list_sessions_returns_created_session() {
    let state = state_with_mock().await;
    let base = spawn_server(state).await;
    let remote = Remote::new(&base, TOKEN).unwrap();

    let id = remote.create_session(Some("plan"), None).await.unwrap();
    let list = remote.list_sessions().await.unwrap();
    assert!(list
        .iter()
        .any(|v| v.get("id").and_then(|i| i.as_str()) == Some(&id)));
}
