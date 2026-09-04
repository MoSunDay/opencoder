//! SSE + terminal-state contract for the DAG control plane: replay ordering
//! with `id:`=seq, `Last-Event-ID` resume without duplicates, the synthetic
//! `run_finished` frame on terminal reports, and the cancel piggyback via
//! heartbeat `cancel_run_ids`. Companion of `dag_api.rs` (split for the
//! 400-line file budget); same harness shape.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use futures::StreamExt;
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, Store};
use serde_json::{json, Value};
use tower::ServiceExt;

type Resp = (StatusCode, Value);

const SPEC: &str = r#"{"name":"etl-demo","steps":[
    {"name":"fetch","kind":{"type":"python","code":"x=1"}},
    {"name":"load","depends_on":["fetch"],"kind":{"type":"python","code":"y=2"}}]}"#;

/// Wrap a raw spec literal in the `DagDefUpsertRequest` envelope.
fn spec_body() -> String {
    format!(r#"{{"spec":{SPEC}}}"#)
}

struct Ctx {
    app: Router,
    store: Arc<dyn Store>,
}

async fn app() -> Ctx {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store: store.clone(),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
        client_override: Some(Arc::new(MockChatClient::new())),
    });
    Ctx {
        app: opencoder_web::build_app(state, None, false),
        store,
    }
}

async fn send(app: &Router, req: Request<Body>) -> Resp {
    let resp = app.clone().oneshot(req).await.expect("router must answer");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({}))
    };
    (status, body)
}

async fn get(app: &Router, uri: &str) -> Resp {
    let r = Request::builder().uri(uri).body(Body::empty()).unwrap();
    send(app, r).await
}

async fn post(app: &Router, uri: &str, body: &str) -> Resp {
    let r = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    send(app, r).await
}

/// POST without a body (cancel, heartbeat, ...).
async fn post0(app: &Router, uri: &str) -> Resp {
    let r = Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    send(app, r).await
}

/// register -> upsert def -> dispatch (pinned) -> claim. Returns (node_id, run_id).
async fn setup_run(app: &Router, name: &str) -> (String, String) {
    let (_, b) = post(
        app,
        "/api/nodes/register",
        &format!(r#"{{"name":"{name}"}}"#),
    )
    .await;
    let node = b["node_id"].as_str().unwrap().to_string();
    let (_, d) = post(app, "/api/dag/defs", &spec_body()).await;
    let def_id = d["id"].as_str().unwrap().to_string();
    let uri = format!("/api/dag/defs/{def_id}/dispatch");
    let (s, r) = post(app, &uri, &format!(r#"{{"node_id":"{node}"}}"#)).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let rid = r["run_id"].as_str().unwrap().to_string();
    let uri = format!("/api/nodes/dag/claim?node_id={node}");
    let (s, c) = get(app, &uri).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    (node, rid)
}

async fn upload(app: &Router, rid: &str, events: Value) -> Resp {
    let body = json!({ "run_id": rid, "events": events }).to_string();
    post(app, &format!("/api/nodes/dag/runs/{rid}/events"), &body).await
}

async fn post_status(app: &Router, rid: &str, status: &str, error: Option<&str>) -> Resp {
    let body = match error {
        Some(e) => json!({"run_id": rid, "status": status, "error": e}),
        None => json!({"run_id": rid, "status": status}),
    };
    post(
        app,
        &format!("/api/nodes/dag/runs/{rid}/status"),
        &body.to_string(),
    )
    .await
}

async fn read_sse(app: &Router, uri: &str, until: &str, last_event_id: Option<i64>) -> String {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(id) = last_event_id {
        builder = builder.header("last-event-id", id.to_string());
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
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

/// SSE text -> (id, event-name, data-json) triples; keep-alive comment lines
/// and empty blocks are skipped.
fn frames(text: &str) -> Vec<(String, String, Value)> {
    text.split("\n\n")
        .filter_map(|block| {
            let (mut id, mut name, mut data) = (String::new(), String::new(), String::new());
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("id: ") {
                    id = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("event: ") {
                    name = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data = v.trim().to_string();
                }
            }
            if name.is_empty() && data.is_empty() {
                None
            } else {
                Some((
                    id,
                    name,
                    serde_json::from_str::<Value>(&data).unwrap_or(json!({})),
                ))
            }
        })
        .collect()
}

// ── SSE replay + resume ────────────────────────────────────────────────────

#[tokio::test]
async fn sse_replays_uploaded_events_in_order_with_id_seq() {
    let ctx = app().await;
    let (_node, rid) = setup_run(&ctx.app, "sse-1").await;
    let (s, b) = upload(
        &ctx.app,
        &rid,
        json!([
            {"kind":"run_started","at_ms":10,"payload":{"n":1}},
            {"kind":"step_started","step":"fetch","at_ms":11},
            {"kind":"step_done","step":"fetch","at_ms":12,"payload":{"ok":true}}
        ]),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let text = read_sse(
        &ctx.app,
        &format!("/api/dag/runs/{rid}/events"),
        "\"ok\":true",
        None,
    )
    .await;
    let fr = frames(&text);
    assert_eq!(fr.len(), 3, "{text}");
    let seqs: Vec<i64> = fr.iter().map(|(id, _, _)| id.parse().unwrap()).collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "ascending seq order: {seqs:?}"
    );
    assert_eq!(fr[0].1, "run_started");
    assert_eq!(fr[1].1, "step_started");
    assert_eq!(fr[1].2["step"], "fetch");
    assert_eq!(fr[2].2["payload"]["ok"], true);
    // The frame data is the full DagEventView (seq echoed inside the JSON).
    assert_eq!(fr[0].2["seq"].as_i64().unwrap(), seqs[0]);

    // Unknown run: fail fast, no hanging empty stream (mirrors sse_nodes).
    let r = Request::builder()
        .uri("/api/dag/runs/01GHOST/events")
        .body(Body::empty())
        .unwrap();
    let resp = ctx.app.clone().oneshot(r).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sse_last_event_id_resume_replays_only_the_tail() {
    let ctx = app().await;
    let (_node, rid) = setup_run(&ctx.app, "sse-2").await;
    upload(
        &ctx.app,
        &rid,
        json!([
            {"kind":"run_started","at_ms":1},
            {"kind":"step_started","step":"fetch","at_ms":2},
            {"kind":"step_done","step":"fetch","at_ms":3}
        ]),
    )
    .await;

    let full = read_sse(
        &ctx.app,
        &format!("/api/dag/runs/{rid}/events"),
        "step_done",
        None,
    )
    .await;
    let fr = frames(&full);
    assert_eq!(fr.len(), 3);

    // Resume from the SECOND frame's id: exactly the tail, no duplicates.
    let cursor: i64 = fr[1].0.parse().unwrap();
    let tail = read_sse(
        &ctx.app,
        &format!("/api/dag/runs/{rid}/events"),
        "step_done",
        Some(cursor),
    )
    .await;
    let tail_fr = frames(&tail);
    assert_eq!(tail_fr.len(), 1, "{tail}");
    assert_eq!(tail_fr[0].1, "step_done");
    assert_eq!(
        tail_fr[0].0.parse::<i64>().unwrap(),
        fr[2].0.parse::<i64>().unwrap()
    );
}

// ── terminal status ────────────────────────────────────────────────────────

#[tokio::test]
async fn status_done_freezes_run_and_emits_run_finished_frame() {
    let ctx = app().await;
    let (_node, rid) = setup_run(&ctx.app, "st-1").await;
    upload(&ctx.app, &rid, json!([{"kind":"run_started","at_ms":1}])).await;

    let (s, b) = post_status(&ctx.app, &rid, "done", None).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let rec = ctx.store.get_dag_run(&rid).await.unwrap().unwrap();
    assert_eq!(rec.status.as_str(), "done");
    assert!(rec.finished_at.is_some());

    // The synthetic terminal frame is durable AND on the stream.
    let uri = format!("/api/dag/runs/{rid}/events");
    let text = read_sse(&ctx.app, &uri, "run_finished", None).await;
    let fr = frames(&text);
    let last = fr.last().expect("frames");
    assert_eq!(last.1, "run_finished");
    assert_eq!(last.2["payload"]["status"], "done");

    // Terminal freeze: a second terminal report is rejected with 409.
    let (s, b) = post_status(&ctx.app, &rid, "error", Some("late")).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
}

// ── cancel + heartbeat piggyback ───────────────────────────────────────────

#[tokio::test]
async fn cancel_collapses_pending_and_piggybacks_running_via_heartbeat() {
    let ctx = app().await;
    // register + def once; both runs below pin to this node.
    let (_, b) = post(&ctx.app, "/api/nodes/register", r#"{"name":"cx-1"}"#).await;
    let node = b["node_id"].as_str().unwrap().to_string();
    let (_, d) = post(&ctx.app, "/api/dag/defs", &spec_body()).await;
    let def_id = d["id"].as_str().unwrap().to_string();
    let dispatch = format!("/api/dag/defs/{def_id}/dispatch");
    let pin = format!(r#"{{"node_id":"{node}"}}"#);

    // (a) pending (dispatched, NOT claimed): cancel collapses it immediately.
    let (_, r1) = post(&ctx.app, &dispatch, &pin).await;
    let rid1 = r1["run_id"].as_str().unwrap().to_string();
    let (s, b) = post0(&ctx.app, &format!("/api/dag/runs/{rid1}/cancel")).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["phase"], "cancelled");
    let rec = ctx.store.get_dag_run(&rid1).await.unwrap().unwrap();
    assert_eq!(rec.status.as_str(), "cancelled");
    // Cancelling a terminal run is refused; unknown id is 404.
    let (s, _) = post0(&ctx.app, &format!("/api/dag/runs/{rid1}/cancel")).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = post0(&ctx.app, "/api/dag/runs/01GHOST/cancel").await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // (b) running (claimed): flips to cancelling and rides the heartbeat.
    let (_, r2) = post(&ctx.app, &dispatch, &pin).await;
    let rid2 = r2["run_id"].as_str().unwrap().to_string();
    let uri = format!("/api/nodes/dag/claim?node_id={node}");
    let (s, _) = get(&ctx.app, &uri).await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = post0(&ctx.app, &format!("/api/dag/runs/{rid2}/cancel")).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["phase"], "cancelling");

    let uri = format!("/api/nodes/{node}/heartbeat");
    let (s, hb) = post0(&ctx.app, &uri).await;
    assert_eq!(s, StatusCode::OK, "{hb}");
    let ids: Vec<&str> = hb["cancel_run_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(ids.contains(&rid2.as_str()), "{hb}");

    // Node aborts and reports; the piggyback drains.
    let (s, _) = post_status(&ctx.app, &rid2, "cancelled", None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, hb) = post0(&ctx.app, &uri).await;
    // Empty `cancel_run_ids` is OMITTED from the wire entirely (serde
    // skip_serializing_if) — absent means "nothing to cancel".
    let ids_empty = hb
        .get("cancel_run_ids")
        .and_then(|v| v.as_array())
        .is_none_or(|a| a.is_empty());
    assert!(ids_empty, "{hb}");
}
