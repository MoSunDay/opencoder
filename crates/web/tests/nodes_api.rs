//! HTTP contract for the node registry half (`/api/nodes*`):
//! signature-auth coverage, register/heartbeat/delete lifecycle, dispatch +
//! synthetic-session isolation, and FIFO claiming through the real router.
//! Mirrors the `app()` harness style of `web_contract.rs`.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::StreamExt;
use opencoder_llm::MockChatClient;
use opencoder_store::{
    EventKind, LibsqlStore, SessionEventRecord, SessionFilter, SessionMeta, Store,
};
use tower::ServiceExt;

mod support;

const TOKEN: &str = "nodes-test-token";

struct Ctx {
    app: axum::Router,
    store: Arc<dyn Store>,
}

async fn app(token: Option<&str>) -> Ctx {
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
        app: opencoder_web::build_app(state, token.map(str::to_string), false),
        store,
    }
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.clone().oneshot(req).await.expect("router must answer");
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

fn req(method: &str, uri: &str, token: Option<&str>, body: Option<String>) -> Request<Body> {
    // Token present → sign (production auth path); absent → raw (negative 401 paths).
    if let Some(t) = token {
        return support::signed_req(method, uri, t, body);
    }
    let b = Request::builder().method(method).uri(uri);
    match body {
        Some(json) => b
            .header("content-type", "application/json")
            .body(Body::from(json)),
        None => b.body(Body::empty()),
    }
    .unwrap()
}

async fn register(app: &axum::Router, name: &str) -> String {
    let (status, body) = send(
        app,
        req(
            "POST",
            "/api/nodes/register",
            Some(TOKEN),
            Some(format!(r#"{{"name":"{name}"}}"#)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["node_id"].as_str().unwrap().to_string()
}

// ── auth (middleware covers the new routes automatically) ─────────────────

#[tokio::test]
async fn nodes_routes_require_token() {
    let ctx = app(Some(TOKEN)).await;
    for uri in [
        "/api/nodes",
        "/api/nodes/tasks/claim?node_id=x",
        "/api/nodes/tasks",
        "/api/nodes/tasks/t1",
        "/api/sessions/s1/task",
    ] {
        let (status, _) = send(&ctx.app, req("GET", uri, None, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri} without token");
    }
    let (status, _) = send(
        &ctx.app,
        req(
            "POST",
            "/api/nodes/register",
            None,
            Some(r#"{"name":"n"}"#.into()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "register without token");
}

/// The positive proof that the layer wraps the fleet routes: a valid bearer
/// passes straight through to the handler (list is empty but 200).
#[tokio::test]
async fn nodes_routes_accept_bearer_token() {
    let ctx = app(Some(TOKEN)).await;
    let (status, body) = send(&ctx.app, req("GET", "/api/nodes", Some(TOKEN), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["nodes"].as_array().unwrap().len(), 0);
}

// ── register / heartbeat / delete ─────────────────────────────────────────

#[tokio::test]
async fn register_then_two_heartbeats_touch_and_delete_invalidates() {
    let ctx = app(None).await;
    let node_id = register(&ctx.app, "worker-a").await;

    // First heartbeat: no cancelling tasks yet → empty cancel list.
    let (s1, hb1) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/{node_id}/heartbeat"),
            None,
            Some("{}".into()),
        ),
    )
    .await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(hb1["cancel_task_ids"], serde_json::json!([]));
    assert!(hb1["server_time_ms"].is_i64());

    // Second heartbeat re-registers nothing; GET shows fresh non-lost status.
    let (_, _) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/{node_id}/heartbeat"),
            None,
            Some("{}".into()),
        ),
    )
    .await;
    let (ls, list) = send(&ctx.app, req("GET", "/api/nodes", None, None)).await;
    assert_eq!(ls, StatusCode::OK);
    let node = &list["nodes"].as_array().unwrap()[0];
    assert_eq!(node["id"], node_id.as_str());
    assert_eq!(node["name"], "worker-a");
    let st = node["status"].as_str().unwrap();
    assert_ne!(st, "lost", "a just-beaten heartbeat must not read lost");
    assert!(st == "idle" || st == "online" || st == "busy");

    // Delete kills the row; subsequent heartbeat is 404.
    let (ds, del) = send(
        &ctx.app,
        req("DELETE", &format!("/api/nodes/{node_id}"), None, None),
    )
    .await;
    assert_eq!(ds, StatusCode::OK, "{del}");
    assert_eq!(del["ok"], serde_json::json!(true));
    let (hs, _) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/{node_id}/heartbeat"),
            None,
            Some("{}".into()),
        ),
    )
    .await;
    assert_eq!(hs, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn duplicate_register_reuses_node_id() {
    let ctx = app(None).await;
    let id1 = register(&ctx.app, "same-name").await;
    let id2 = register(&ctx.app, "same-name").await;
    assert_eq!(id1, id2, "stable id keeps dispatched tasks dangling-free");
}

// ── dispatch + synthetic session isolation ────────────────────────────────

#[tokio::test]
async fn dispatch_creates_task_and_hidden_synthetic_session() {
    let ctx = app(None).await;
    let node_id = register(&ctx.app, "w").await;

    let (s, disp) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/{node_id}/tasks"),
            None,
            Some(r#"{"prompt":"run lint","title":"lint"}"#.into()),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{disp}");
    let task_id = disp["task_id"].as_str().unwrap().to_string();
    let sid = disp["session_id"].as_str().unwrap().to_string();
    assert_ne!(task_id, sid);

    // Queue lists the pending task; synthetic session exists but is hidden…
    let (ts, tasks) = send(
        &ctx.app,
        req("GET", &format!("/api/nodes/{node_id}/tasks"), None, None),
    )
    .await;
    assert_eq!(ts, StatusCode::OK);
    let task = &tasks["tasks"].as_array().unwrap()[0];
    assert_eq!(task["id"], task_id.as_str());
    assert_eq!(task["status"], "pending");
    assert_eq!(task["session_id"], sid.as_str());
    assert!(ctx.store.get_session(&sid).await.unwrap().is_some());

    // …from listings at both settings.
    for include in [false, true] {
        let listed = ctx
            .store
            .list_sessions(&SessionFilter {
                limit: 50,
                include_subagents: include,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(
            !listed.iter().any(|i| i.id == sid),
            "node session leaked into listing (include_subagents={include})"
        );
    }

    // Mutating the synthetic session via the regular session API is a 409.
    for (method, uri, body) in [
        (
            "POST",
            format!("/api/sessions/{sid}/prompt"),
            Some(r#"{"prompt":"hi"}"#.to_string()),
        ),
        (
            "POST",
            format!("/api/sessions/{sid}/agent"),
            Some(r#"{"value":"plan"}"#.to_string()),
        ),
        (
            "POST",
            format!("/api/sessions/{sid}/model"),
            Some(r#"{"value":"m2"}"#.to_string()),
        ),
        ("POST", format!("/api/sessions/{sid}/interrupt"), None),
        ("POST", format!("/api/sessions/{sid}/fork"), None),
        ("POST", format!("/api/sessions/{sid}/compact"), None),
        ("POST", format!("/api/sessions/{sid}/handoff"), None),
        (
            "POST",
            format!("/api/sessions/{sid}/skill"),
            Some(r#"{"skill":"go"}"#.to_string()),
        ),
    ] {
        let (ms, mb) = send(&ctx.app, req(method, &uri, None, body)).await;
        assert_eq!(ms, StatusCode::CONFLICT, "{method} {uri}: {mb}");
    }

    // Empty prompt dispatch is refused up front.
    let (es, eb) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/{node_id}/tasks"),
            None,
            Some(r#"{"prompt":"  "}\n"#.into()),
        ),
    )
    .await;
    assert_eq!(es, StatusCode::BAD_REQUEST, "{eb}");

    // Dispatching to an unknown node is 404, and event streams of unknown
    // tasks are 404 too.
    let (us, _) = send(
        &ctx.app,
        req(
            "POST",
            "/api/nodes/no-such/tasks",
            None,
            Some(r#"{"prompt":"x"}"#.into()),
        ),
    )
    .await;
    assert_eq!(us, StatusCode::NOT_FOUND);
    let (ufs, _) = send(
        &ctx.app,
        req(
            "GET",
            &format!("/api/nodes/tasks/{task_id}-ghost/events"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(ufs, StatusCode::NOT_FOUND);
}

// ── claim FIFO over HTTP ──────────────────────────────────────────────────

#[tokio::test]
async fn claim_fifo_over_http_then_empty_is_204() {
    let ctx = app(None).await;
    let node_id = register(&ctx.app, "fifo").await;
    let mut ids = Vec::new();
    for i in 0..2 {
        let (_, d) = send(
            &ctx.app,
            req(
                "POST",
                &format!("/api/nodes/{node_id}/tasks"),
                None,
                Some(format!(r#"{{"prompt":"job-{i}"}}"#)),
            ),
        )
        .await;
        ids.push(d["task_id"].as_str().unwrap().to_string());
    }

    // The queue enforces single-active-task: claim -> finish -> next claim.
    // FIFO order therefore shows up as job-0 strictly before job-1.
    for expected in &ids {
        let (cs, claim) = send(
            &ctx.app,
            req(
                "GET",
                &format!("/api/nodes/tasks/claim?node_id={node_id}"),
                None,
                None,
            ),
        )
        .await;
        assert_eq!(cs, StatusCode::OK);
        assert_eq!(
            claim["task"]["task_id"],
            expected.as_str(),
            "claims follow FIFO order"
        );
        assert_eq!(
            claim["task"]["prompt"],
            format!("job-{}", ids.iter().position(|i| i == expected).unwrap())
        );
        assert!(claim["task"]["session_id"].as_str().is_some());
        assert!(claim["control"].is_null());

        let (_, rep) = send(
            &ctx.app,
            req(
                "POST",
                &format!("/api/nodes/tasks/{expected}/status"),
                None,
                Some(r#"{"status":"done"}"#.into()),
            ),
        )
        .await;
        assert_eq!(rep["ok"], serde_json::json!(true));
    }

    // Both tasks terminal → nothing due.
    let (es, empty_body) = send(
        &ctx.app,
        req(
            "GET",
            &format!("/api/nodes/tasks/claim?node_id={node_id}"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(es, StatusCode::NO_CONTENT, "{empty_body}");
}

// ── task status pulls (single / fleet-wide / session reverse lookup) ──────

async fn dispatch_ok(app: &axum::Router, node_id: &str) -> (String, String) {
    let (s, d) = send(
        app,
        req(
            "POST",
            &format!("/api/nodes/{node_id}/tasks"),
            None,
            Some(format!(r#"{{"prompt":"job on {node_id}"}}"#)),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{d}");
    // Additive submit contract: status rides along so callers can render
    // without a follow-up fetch.
    assert_eq!(d["status"], "pending", "{d}");
    (
        d["task_id"].as_str().unwrap().to_string(),
        d["session_id"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn task_detail_returns_record_with_sse_bootstrap_seq() {
    let ctx = app(None).await;
    let node_id = register(&ctx.app, "detail").await;
    let (task_id, sid) = dispatch_ok(&ctx.app, &node_id).await;

    let (s, t) = send(
        &ctx.app,
        req("GET", &format!("/api/nodes/tasks/{task_id}"), None, None),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["id"], task_id.as_str());
    assert_eq!(t["node_id"], node_id.as_str());
    assert_eq!(t["session_id"], sid.as_str());
    assert_eq!(t["status"], "pending");
    assert_eq!(t["last_event_seq"], 0, "no events persisted yet");

    // Unknown ids are 404.
    let (s404, b404) = send(
        &ctx.app,
        req("GET", "/api/nodes/tasks/no-such-task", None, None),
    )
    .await;
    assert_eq!(s404, StatusCode::NOT_FOUND, "{b404}");
}

#[tokio::test]
async fn task_detail_tracks_events_and_terminal_closure() {
    let ctx = app(None).await;
    let node_id = register(&ctx.app, "closer").await;
    let (task_id, _) = dispatch_ok(&ctx.app, &node_id).await;

    // Claim -> running, upload one event, then report done (closure event).
    let (cs, _) = send(
        &ctx.app,
        req(
            "GET",
            &format!("/api/nodes/tasks/claim?node_id={node_id}"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(cs, StatusCode::OK);
    let (es, eb) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/tasks/{task_id}/events"),
            None,
            Some(r#"{"events":[{"sse_kind":"text_delta","payload":{"text":"hi"},"ts":1}]}"#.into()),
        ),
    )
    .await;
    assert_eq!(es, StatusCode::OK, "{eb}");
    let (rs, rb) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/tasks/{task_id}/status"),
            None,
            Some(r#"{"status":"done"}"#.into()),
        ),
    )
    .await;
    assert_eq!(rs, StatusCode::OK, "{rb}");

    let (_, t) = send(
        &ctx.app,
        req("GET", &format!("/api/nodes/tasks/{task_id}"), None, None),
    )
    .await;
    assert_eq!(t["status"], "done", "terminal freeze stays readable");
    let seq = t["last_event_seq"].as_i64().unwrap();
    assert!(
        seq >= 2,
        "uploaded event + terminal closure must be counted, got {seq}"
    );
}

#[tokio::test]
async fn fleet_task_list_filters_sorts_and_limits() {
    let ctx = app(None).await;
    let a = register(&ctx.app, "fl-a").await;
    let b = register(&ctx.app, "fl-b").await;
    let (a1, _) = dispatch_ok(&ctx.app, &a).await;
    dispatch_ok(&ctx.app, &a).await;
    let (b1, _) = dispatch_ok(&ctx.app, &b).await;

    // Drive a1 and b1 terminal so the status filter has something to bite.
    for (node, tid) in [(&a, &a1), (&b, &b1)] {
        let (cs, _) = send(
            &ctx.app,
            req(
                "GET",
                &format!("/api/nodes/tasks/claim?node_id={node}"),
                None,
                None,
            ),
        )
        .await;
        assert_eq!(cs, StatusCode::OK);
        let (rs, rb) = send(
            &ctx.app,
            req(
                "POST",
                &format!("/api/nodes/tasks/{tid}/status"),
                None,
                Some(r#"{"status":"done"}"#.into()),
            ),
        )
        .await;
        assert_eq!(rs, StatusCode::OK, "{rb}");
    }

    // Unfiltered: every task, fleet-wide, FIFO (oldest first).
    let (s, all) = send(&ctx.app, req("GET", "/api/nodes/tasks", None, None)).await;
    assert_eq!(s, StatusCode::OK, "{all}");
    let ids: Vec<&str> = all["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], a1, "oldest dispatch first");

    // node_id filter only.
    let (_, only_a) = send(
        &ctx.app,
        req("GET", &format!("/api/nodes/tasks?node_id={a}"), None, None),
    )
    .await;
    let arr = only_a["tasks"].as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert!(arr.iter().all(|t| t["node_id"] == a.as_str()));

    // status filter only.
    let (_, done) = send(
        &ctx.app,
        req("GET", "/api/nodes/tasks?status=done", None, None),
    )
    .await;
    let arr = done["tasks"].as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert!(arr.iter().all(|t| t["status"] == "done"));

    // Combined node_id + status.
    let (_, a_done) = send(
        &ctx.app,
        req(
            "GET",
            &format!("/api/nodes/tasks?node_id={a}&status=done"),
            None,
            None,
        ),
    )
    .await;
    let arr = a_done["tasks"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], a1.as_str());

    // limit keeps the FIFO head.
    let (_, lim) = send(&ctx.app, req("GET", "/api/nodes/tasks?limit=1", None, None)).await;
    let arr = lim["tasks"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], a1.as_str());

    // Unknown status is a 400, never a silently-empty 200.
    let (s400, b400) = send(
        &ctx.app,
        req("GET", "/api/nodes/tasks?status=quantum", None, None),
    )
    .await;
    assert_eq!(s400, StatusCode::BAD_REQUEST, "{b400}");
}

#[tokio::test]
async fn static_claim_route_outranks_task_detail() {
    let ctx = app(None).await;
    let node_id = register(&ctx.app, "prio").await;
    dispatch_ok(&ctx.app, &node_id).await;

    // If `:tid` captured the literal "claim", workers would get a task-detail
    // JSON body instead of the claim envelope.
    let (s, claim) = send(
        &ctx.app,
        req(
            "GET",
            &format!("/api/nodes/tasks/claim?node_id={node_id}"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{claim}");
    assert!(
        claim["task"]["task_id"].as_str().is_some(),
        "static /claim must win over :tid, got {claim}"
    );
}

#[tokio::test]
async fn session_reverse_lookup_resolves_and_404s_ordinary_sessions() {
    let ctx = app(None).await;
    let node_id = register(&ctx.app, "rev").await;
    let (task_id, sid) = dispatch_ok(&ctx.app, &node_id).await;

    let (s, t) = send(
        &ctx.app,
        req("GET", &format!("/api/sessions/{sid}/task"), None, None),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["id"], task_id.as_str());
    assert_eq!(t["session_id"], sid.as_str());

    // Ordinary session: legal no-task case.
    ctx.store
        .create_session(&SessionMeta {
            id: "plain".into(),
            ..SessionMeta::default()
        })
        .await
        .unwrap();
    let (s404, b404) = send(&ctx.app, req("GET", "/api/sessions/plain/task", None, None)).await;
    assert_eq!(s404, StatusCode::NOT_FOUND, "{b404}");
}

// ── node-task SSE `id:` field (F4) ──────────────────────────────────────────

/// Bounded read of a streamed SSE body (the stream itself never ends; the
/// budget must, or the test would hang on the parked hub receiver). Mirrors
/// the helper in `web_list_events.rs`.
async fn read_sse_text(resp: axum::response::Response, until: &str) -> String {
    let mut stream = resp.into_body().into_data_stream();
    let mut text = String::new();
    for _ in 0..40 {
        match tokio::time::timeout(std::time::Duration::from_millis(300), stream.next()).await {
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

/// Persisted task-session events must replay with `id:` equal to their store
/// seq — the reconnect cursor `Last-Event-ID` resumes from on this endpoint.
#[tokio::test]
async fn node_task_events_frames_carry_seq_as_sse_id() {
    let ctx = app(None).await;
    let node_id = register(&ctx.app, "sse-id").await;
    let (_, disp) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/{node_id}/tasks"),
            None,
            Some(r#"{"prompt":"run lint"}"#.into()),
        ),
    )
    .await;
    let tid = disp["task_id"].as_str().unwrap().to_string();
    let sid = disp["session_id"].as_str().unwrap().to_string();

    // A persisted row (store assigns the seq) — the replay source.
    ctx.store
        .append_event(&SessionEventRecord {
            session_id: sid.clone(),
            kind: EventKind::Step,
            payload: serde_json::json!({ "worker": true }),
            ts: 1,
            seq: None,
            sse_kind: Some("status".into()),
        })
        .await
        .unwrap();
    let seq_of_payload: std::collections::HashMap<String, i64> = ctx
        .store
        .events_after(&sid, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|r| {
            (
                serde_json::to_string(&r.payload).unwrap(),
                r.seq.expect("persisted row carries its seq"),
            )
        })
        .collect();

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/nodes/tasks/{tid}/events?after=0"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let text = read_sse_text(resp, "\"worker\":true").await;

    // Every data block's `id:` must be the seq of the very payload streamed.
    let mut checked = 0;
    for block in text.split("\n\n") {
        let Some(data) = block.lines().find_map(|l| l.strip_prefix("data: ")) else {
            continue; // keep-alive comment blocks carry no data line
        };
        let payload =
            serde_json::to_string(&serde_json::from_str::<serde_json::Value>(data.trim()).unwrap())
                .unwrap();
        let seq = seq_of_payload
            .get(&payload)
            .unwrap_or_else(|| panic!("streamed payload must be a persisted one: {data}"));
        let id_line = block
            .lines()
            .find_map(|l| l.strip_prefix("id: "))
            .unwrap_or_else(|| panic!("frame must carry an id: line, got: {block}"));
        assert_eq!(
            id_line.trim().parse::<i64>().unwrap(),
            *seq,
            "SSE id must equal the persisted event seq"
        );
        checked += 1;
    }
    assert!(checked >= 1, "replayed frame must be id-tagged: {text}");
}
