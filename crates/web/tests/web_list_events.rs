//! Integration tests for session-list filtering (`?workdir=`) and
//! `Last-Event-ID` SSE replay.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::StreamExt;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{EventKind, LibsqlStore, SessionEventRecord, Store};
use tower::ServiceExt;

struct Ctx {
    app: axum::Router,
    store: Arc<dyn Store>,
    workdir: std::path::PathBuf,
}

async fn app() -> Ctx {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
    let mock = MockChatClient::new().with_default(vec![LlmEvent::Completed {
        text: "t".into(),
        tool_calls: vec![],
        usage: None,
    }]);
    let state = Arc::new(opencoder_web::AppState {
        store: store.clone(),
        workdir: workdir.clone(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        client_override: Some(Arc::new(mock) as Arc<dyn ChatStream>),
    });
    Ctx {
        app: opencoder_web::build_app(state, None, false),
        store,
        workdir,
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

async fn get_json(ctx: &Ctx, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = ctx
        .app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

async fn read_sse_text(resp: axum::response::Response, until: &str) -> String {
    let mut stream = resp.into_body().into_data_stream();
    let mut text = String::new();
    for _ in 0..40 {
        match tokio::time::timeout(Duration::from_millis(300), stream.next()).await {
            Ok(Some(Ok(bytes))) => {
                text.push_str(&String::from_utf8_lossy(&bytes));
                if text.contains(until) {
                    break;
                }
            }
            _ => break,
        }
    }
    text
}

// ── workdir filter ───────────────────────────────────────────────────────

/// Sessions created by this server carry its workdir hash; the list filter
/// must match on it and exclude sessions from a different workdir.
#[tokio::test]
async fn session_list_filters_by_workdir_hash() {
    let ctx = app().await;
    let a = create_session(&ctx).await;
    let b = create_session(&ctx).await;

    // Same workdir → both sessions listed.
    let wd = ctx.workdir.to_string_lossy().to_string();
    let (status, body) = get_json(&ctx, &format!("/api/sessions?workdir={wd}")).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert!(ids.contains(&a.as_str()), "session a must match: {ids:?}");
    assert!(ids.contains(&b.as_str()), "session b must match: {ids:?}");

    // Nonexistent workdir → nothing.
    let (status, body) = get_json(&ctx, "/api/sessions?workdir=/nonexistent-dir-xyz").await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert!(
        !ids.contains(&a.as_str()) && !ids.contains(&b.as_str()),
        "foreign workdir must not match: {ids:?}"
    );
}

// ── Last-Event-ID replay ────────────────────────────────────────────────

/// Seed two persisted events, then request the stream with the SSE-standard
/// `Last-Event-ID: 0` header (no `?after=`): the replay window must be
/// delivered. An unparseable header is ignored (falls back to 0 → replay).
#[tokio::test]
async fn last_event_id_header_drives_replay() {
    let ctx = app().await;
    let id = create_session(&ctx).await;
    for i in 0..2 {
        ctx.store
            .append_event(&SessionEventRecord {
                session_id: id.clone(),
                kind: EventKind::Step,
                payload: serde_json::json!({ "i": i }),
                ts: i,
                seq: None,
                sse_kind: Some("status".into()),
            })
            .await
            .unwrap();
    }

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{id}/events"))
                .header("last-event-id", "0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let text = read_sse_text(resp, "\"i\":1").await;
    assert!(
        text.contains("\"i\":0") && text.contains("\"i\":1"),
        "Last-Event-ID: 0 must replay both events; got: {text}"
    );

    // Garbage header value must not break the stream (ignored → replay).
    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{id}/events"))
                .header("last-event-id", "not-a-number")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let text = read_sse_text(resp, "\"i\":1").await;
    assert!(
        text.contains("\"i\":0"),
        "invalid Last-Event-ID must fall back to replay-from-0; got: {text}"
    );
}
