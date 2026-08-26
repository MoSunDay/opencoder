//! HTTP contract for the node registry half (`/api/nodes*`):
//! bearer-token coverage, register/heartbeat/delete lifecycle, dispatch +
//! synthetic-session isolation, and FIFO claiming through the real router.
//! Mirrors the `app()` harness style of `web_contract.rs`.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, SessionFilter, Store};
use tower::ServiceExt;

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
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
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
    for uri in ["/api/nodes", "/api/nodes/tasks/claim?node_id=x"] {
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
            claim["task_id"],
            expected.as_str(),
            "claims follow FIFO order"
        );
        assert_eq!(
            claim["prompt"],
            format!("job-{}", ids.iter().position(|i| i == expected).unwrap())
        );
        assert!(claim["session_id"].as_str().is_some());

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
