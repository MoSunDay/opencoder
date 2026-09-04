//! `/api/brain/plans` + `/api/brain/dispatch` REST contract tests, driven
//! through the production `build_app` router. The brain runtime is backed by
//! a SHARED `MockChatClient` (script queued from the test, embed = pure
//! hash), so the planner LLM call is fully scripted: identical texts embed
//! identically (cosine 1.0) and a threshold of 0.98 makes branch routing
//! deterministic.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::json;
use tower::ServiceExt;

use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

const TOPIC_A: &str = "db migration plan";
const TOPIC_B: &str = "write unit tests";
const THRESH: f64 = 0.98;

fn capability_payload(summary: &str) -> serde_json::Value {
    json!({
        "capability_type": "tool-usage",
        "summary": summary,
        "input_desc": "a work request",
        "output_desc": "completed work",
        "eng_inputs": ["exemplar input"],
    })
}

/// App state wired to a shared scripted mock — the SAME instance the test
/// queues planner scripts onto, which is the whole point (the runtime's
/// client must be reachable before the app is built).
async fn state() -> (Arc<opencoder_web::AppState>, Arc<MockChatClient>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(MockChatClient::new());
    let client: Arc<dyn ChatStream> = mock.clone();
    let brain = opencoder_brain::Runtime::new(store.clone(), client, "mock-embed")
        .with_chat_model("planner-chat");
    (
        Arc::new(opencoder_web::AppState {
            brain,
            store,
            workdir: std::env::temp_dir(),
            handles: opencoder_web::handle::new_handle_map(),
            nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
            controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
            team: opencoder_web::team_state::mock(),
            project: opencoder_web::ProjectService::new(),
            client_override: None,
        }),
        mock,
    )
}

fn app(state: Arc<opencoder_web::AppState>) -> Router {
    opencoder_web::build_app(state, None, false)
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => req
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.expect("router must answer");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({}))
    };
    (status, body)
}

fn queue_tree(mock: &MockChatClient, cap_a: &str, cap_b: &str) {
    let reply = format!(
        "{{\"threshold\":{THRESH},\"root\":{{\"id\":\"b1\",\"kind\":\"branch\",\"topic\":\"{TOPIC_A}\",\"yes\":{{\"id\":\"l1\",\"kind\":\"leaf\",\"capability_id\":\"{cap_a}\",\"reason\":\"db work\"}},\"no\":{{\"id\":\"l2\",\"kind\":\"leaf\",\"capability_id\":\"{cap_b}\",\"reason\":\"test work\"}}}}}}"
    );
    mock.queue_script(vec![
        LlmEvent::TextDelta(reply.clone()),
        LlmEvent::Completed {
            text: reply,
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);
}

/// Seed two capabilities through the public REST surface, returning ids.
async fn seed_caps(app: &Router) -> [String; 2] {
    let mut ids = Vec::new();
    for summary in [TOPIC_A, TOPIC_B] {
        let (st, body) = call(
            app,
            "POST",
            "/api/brain/capabilities",
            Some(capability_payload(summary)),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{body}");
        ids.push(body["capability"]["id"].as_str().unwrap().to_string());
    }
    [ids.pop().unwrap(), ids.pop().unwrap()]
}

#[tokio::test]
async fn plan_then_dispatch_through_plan_id_roundtrip() {
    let (state, mock) = state().await;
    let app = app(state.clone());
    let [cap_a, cap_b] = seed_caps(&app).await;
    queue_tree(&mock, &cap_a, &cap_b);

    // POST /plans → 201 with the persisted record + parsed tree.
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/plans",
        Some(json!({ "situation": TOPIC_A })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{body}");
    assert!(body["plan"]["id"]
        .as_str()
        .unwrap()
        .starts_with("brain-plan-"));
    assert_eq!(body["plan"]["chat_model"], "planner-chat");
    assert_eq!(body["tree"]["root"]["kind"], "branch");
    let plan_id = body["plan"]["id"].as_str().unwrap().to_string();

    // GET /plans/:id → 200; unknown id → 404.
    let (st, body) = call(&app, "GET", &format!("/api/brain/plans/{plan_id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["plan"]["id"].as_str(), Some(plan_id.as_str()));
    let (st, body) = call(&app, "GET", "/api/brain/plans/brain-plan-none", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{body}");

    // POST /dispatch { plan_id }: situation equals the branch topic → cap A.
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/dispatch",
        Some(json!({ "situation": TOPIC_A, "plan_id": plan_id })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["capability_id"].as_str(), Some(cap_a.as_str()));
    assert_eq!(body["path"].as_array().unwrap().len(), 2);
    // Anything else → the no leaf.
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/dispatch",
        Some(json!({ "situation": "unrelated incident", "plan_id": plan_id })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["capability_id"].as_str(), Some(cap_b.as_str()));
}

#[tokio::test]
async fn dispatch_without_plan_id_plans_then_reuses_the_digest_cache() {
    let (state, mock) = state().await;
    let app = app(state.clone());
    let [cap_a, cap_b] = seed_caps(&app).await;
    queue_tree(&mock, &cap_a, &cap_b);

    // First dispatch: nothing cached → plans first (script consumed).
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/dispatch",
        Some(json!({ "situation": TOPIC_A })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["planned_fresh"], true);
    assert_eq!(body["capability_id"].as_str(), Some(cap_a.as_str()));

    // Second dispatch, same situation: NO script queued — the cached plan
    // must answer without an LLM call.
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/dispatch",
        Some(json!({ "situation": TOPIC_A })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["planned_fresh"], false);
    assert_eq!(body["capability_id"].as_str(), Some(cap_a.as_str()));
}

#[tokio::test]
async fn error_contract_400_404_502() {
    let (state, mock) = state().await;
    let app = app(state.clone());
    let [_cap_a, _cap_b] = seed_caps(&app).await;

    // Empty situation → 400.
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/plans",
        Some(json!({ "situation": "   " })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");

    // Unparseable planner reply → typed marker → 502.
    mock.queue_script(vec![LlmEvent::Completed {
        text: "no json today".to_string(),
        tool_calls: Vec::new(),
        usage: None,
    }]);
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/plans",
        Some(json!({ "situation": TOPIC_A })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{body}");

    // Dispatch against an unknown plan id → 404.
    let (st, body) = call(
        &app,
        "POST",
        "/api/brain/dispatch",
        Some(json!({ "situation": TOPIC_A, "plan_id": "brain-plan-ghost" })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{body}");
}

/// Brain routes live under `/api`, so they inherit the HMAC signature gate:
/// a token-protected app must reject an unsigned dispatch with 401.
#[tokio::test]
async fn unsigned_dispatch_request_is_401() {
    let (state, _mock) = state().await;
    let app = opencoder_web::build_app(state, Some("sekret-token-123".to_string()), false);
    let (st, _body) = call(
        &app,
        "POST",
        "/api/brain/dispatch",
        Some(json!({ "situation": TOPIC_A })),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}
