//! HTTP contract for node-task operations (`/api/nodes/tasks*` + cancel):
//! live-event upload persistence/replay, terminal-status convergence, and
//! cancellation flow for pending vs running tasks. Harness mirrors
//! `web_list_events.rs` (SSE frames are read with a read-timeout loop).

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::StreamExt;
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, Store};
use tower::ServiceExt;

struct Ctx {
    app: axum::Router,
    store: Arc<dyn Store>,
}

async fn app() -> Ctx {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        store: store.clone(),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        client_override: Some(Arc::new(MockChatClient::new())),
    });
    Ctx {
        app: opencoder_web::build_app(state, None, false),
        store,
    }
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.clone().oneshot(req).await.expect("router answers");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}))
    };
    (status, body)
}

fn req(method: &str, uri: &str, body: Option<String>) -> Request<Body> {
    match body {
        Some(json) => Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(json)),
        None => Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty()),
    }
    .unwrap()
}

async fn register(app: &axum::Router, name: &str) -> String {
    let (_, b) = send(
        app,
        req(
            "POST",
            "/api/nodes/register",
            Some(format!(r#"{{"name":"{name}"}}"#)),
        ),
    )
    .await;
    b["node_id"].as_str().unwrap().into()
}

/// Dispatch one task; returns (task_id, session_id).
async fn dispatch(ctx: &Ctx, node_id: &str, prompt: &str) -> (String, String) {
    let (_, d) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/{node_id}/tasks"),
            Some(format!(r#"{{"prompt":"{prompt}"}}"#)),
        ),
    )
    .await;
    (
        d["task_id"].as_str().unwrap().into(),
        d["session_id"].as_str().unwrap().into(),
    )
}

async fn claim_task(ctx: &Ctx, node_id: &str) -> Option<(String, String)> {
    let (s, c) = send(
        &ctx.app,
        req(
            "GET",
            &format!("/api/nodes/tasks/claim?node_id={node_id}"),
            None,
        ),
    )
    .await;
    if s == StatusCode::NO_CONTENT {
        return None;
    }
    assert_eq!(s, StatusCode::OK);
    Some((
        c["task"]["task_id"].as_str().unwrap().into(),
        c["task"]["session_id"].as_str().unwrap().into(),
    ))
}

/// Upload `evts` to a claimed task. Each triple is (sse_kind, payload, ts).
async fn upload(
    ctx: &Ctx,
    tid: &str,
    evts: &[(&str, serde_json::Value, i64)],
) -> (StatusCode, serde_json::Value) {
    let events: Vec<serde_json::Value> = evts
        .iter()
        .map(|(k, p, ts)| serde_json::json!({ "sse_kind": k, "payload": p, "ts": ts }))
        .collect();
    send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/tasks/{tid}/events"),
            Some(serde_json::json!({ "events": events }).to_string()),
        ),
    )
    .await
}

/// Collect SSE wire text until `until` appears in the stream.
async fn read_sse_text(resp: axum::response::Response, until: &str) -> String {
    let mut stream = resp.into_body().into_data_stream();
    let mut text = String::new();
    for _ in 0..40 {
        match tokio::time::timeout(Duration::from_millis(200), stream.next()).await {
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

/// Parse SSE wire text into (event-name, data-json) pairs.
fn frames(text: &str) -> Vec<(String, serde_json::Value)> {
    text.split("\n\n")
        .filter_map(|block| {
            let mut name = String::new();
            let mut data = String::new();
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("event: ") {
                    name = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data = v.trim().to_string();
                }
            }
            if name.is_empty() && data.is_empty() {
                None
            } else {
                Some((
                    name,
                    serde_json::from_str::<serde_json::Value>(&data)
                        .unwrap_or(serde_json::json!({})),
                ))
            }
        })
        .collect()
}

// ── events: upload → persist → replay ─────────────────────────────────────

#[tokio::test]
async fn uploaded_events_replay_with_monotonic_seqs_and_cursor() {
    let ctx = app().await;
    let node = register(&ctx.app, "ev").await;
    let (tid, sid) = dispatch(&ctx, &node, "stream me").await;
    // Claim so uploads pass the running-gate.
    assert!(claim_task(&ctx, &node).await.is_some());

    // 2× identical TextDelta payload + 1 ToolStart.
    let (_, up) = upload(
        &ctx,
        &tid,
        &[
            ("text_delta", serde_json::json!({ "delta": "a" }), 1),
            ("text_delta", serde_json::json!({ "delta": "a" }), 2),
            ("tool_start", serde_json::json!({ "tool": "grep" }), 3),
        ],
    )
    .await;
    assert_eq!(up["appended"], serde_json::json!(3));

    // Store-side reconciliation: 3 rows, strictly increasing seqs.
    let all = ctx.store.events_after(&sid, 0).await.unwrap();
    assert_eq!(all.len(), 3);
    let seqs: Vec<i64> = all.iter().map(|e| e.seq.unwrap()).collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "seqs must be monotonic: {seqs:?}"
    );

    // Full replay: exactly 3 frames with the right SSE names (identical
    // TextDelta payloads must NOT be collapsed — seq-dedup is exact).
    let resp = ctx
        .app
        .clone()
        .oneshot(req("GET", &format!("/api/nodes/tasks/{tid}/events"), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let fr = frames(&read_sse_text(resp, "\"tool\":\"grep\"").await);
    assert_eq!(fr.len(), 3, "expected 3 frames, got {fr:?}");
    assert_eq!(fr[0].0, "text_delta");
    assert_eq!(fr[1].0, "text_delta");
    assert_eq!(fr[2].0, "tool_start");

    // Cursor resume after the second event: only frame 3 remains — on the wire…
    let seq2 = seqs[1];
    let resp = ctx
        .app
        .clone()
        .oneshot(req(
            "GET",
            &format!("/api/nodes/tasks/{tid}/events?after={seq2}"),
            None,
        ))
        .await
        .unwrap();
    let fr2 = frames(&read_sse_text(resp, "\"tool\":\"grep\"").await);
    assert_eq!(
        fr2.len(),
        1,
        "after={seq2} must yield only frame 3: {fr2:?}"
    );
    assert_eq!(fr2[0].0, "tool_start");

    // …and in the store (no loss either way).
    let tail = ctx.store.events_after(&sid, seq2).await.unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].sse_kind.as_deref(), Some("tool_start"));

    // Last-Event-ID header equivalent works too.
    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/nodes/tasks/{tid}/events"))
                .header("last-event-id", seq2.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let fr3 = frames(&read_sse_text(resp, "\"tool\":\"grep\"").await);
    assert_eq!(fr3.len(), 1);

    // Empty batch appends nothing but stays legal on a running task.
    let (_, empty) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/tasks/{tid}/events"),
            Some(r#"{"events":[]}"#.into()),
        ),
    )
    .await;
    assert_eq!(empty["appended"], serde_json::json!(0));
}

// ── guard rails ───────────────────────────────────────────────────────────

#[tokio::test]
async fn upload_to_non_running_task_is_409_and_empty_claim_is_204() {
    let ctx = app().await;
    let node = register(&ctx.app, "g").await;

    // Nothing dispatched yet: claim is 204.
    let (cs, _) = send(
        &ctx.app,
        req(
            "GET",
            &format!("/api/nodes/tasks/claim?node_id={node}"),
            None,
        ),
    )
    .await;
    assert_eq!(cs, StatusCode::NO_CONTENT);

    // A still-pending task rejects uploads (running gate) with 409.
    let (tid, _sid) = dispatch(&ctx, &node, "queued only").await;
    let (us, ub) = upload(&ctx, &tid, &[("text_delta", serde_json::json!({}), 1)]).await;
    assert_eq!(us, StatusCode::CONFLICT, "{ub}");

    // An unknown task id is 404 rather than 409: nothing to gate at all.
    let (ks, kb) = send(
        &ctx.app,
        req(
            "POST",
            "/api/nodes/tasks/01GHOST01/events",
            Some(r#"{"events":[{"sse_kind":"text_delta","payload":{},"ts":1}]}"#.into()),
        ),
    )
    .await;
    assert_eq!(ks, StatusCode::NOT_FOUND, "{kb}");
}

#[tokio::test]
async fn status_report_converges_task_and_stream_tail_seq() {
    let ctx = app().await;
    let node = register(&ctx.app, "st").await;
    let (tid, sid) = dispatch(&ctx, &node, "wrap up").await;
    assert!(claim_task(&ctx, &node).await.is_some());
    upload(
        &ctx,
        &tid,
        &[("text_delta", serde_json::json!({ "delta": "z" }), 9)],
    )
    .await;

    let (ss, sb) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/tasks/{tid}/status"),
            Some(r#"{"status":"done"}"#.into()),
        ),
    )
    .await;
    assert_eq!(ss, StatusCode::OK, "{sb}");
    assert_eq!(sb["status"], "done");

    // Task reached its terminal state in the store…
    let rec = ctx.store.get_node_task(&tid).await.unwrap().unwrap();
    assert_eq!(rec.status.as_str(), "done");
    assert!(rec.finished_at.is_some());

    // …and the stream's tail frame is `done`, carrying the same seq the store
    // recorded for that closure event (wire ↔ store reconciliation).
    let last_store_seq = ctx.store.last_event_seq(&sid).await.unwrap();
    let resp = ctx
        .app
        .clone()
        .oneshot(req("GET", &format!("/api/nodes/tasks/{tid}/events"), None))
        .await
        .unwrap();
    let fr = frames(&read_sse_text(resp, "\"ok\":true").await);
    let last = fr.last().expect("closure frame");
    assert_eq!(last.0, "done");
    assert_eq!(last.1["task_id"], tid.as_str());
    assert_eq!(last.1["ok"], serde_json::json!(true));
    // Frames carry no id:, so reconcile via row count: the store has exactly
    // one more event than the upload (the closure), and the broadcast was the
    // closure itself — any further status write would be an illegal transition.
    assert_eq!(last_store_seq, 2, "1 upload + 1 closure event");

    // Double-terminal report is rejected as a conflict.
    let (ds, db) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/tasks/{tid}/status"),
            Some(r#"{"status":"error","error":"late"}"#.into()),
        ),
    )
    .await;
    assert_eq!(ds, StatusCode::CONFLICT, "{db}");

    // Invalid status literals are a plain 400.
    let (bs, _) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/tasks/{tid}/status"),
            Some(r#"{"status":"Done"}"#.into()),
        ),
    )
    .await;
    assert_eq!(bs, StatusCode::BAD_REQUEST);
}
