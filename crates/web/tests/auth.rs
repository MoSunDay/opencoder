//! Signature-auth middleware contract.
//!
//! Every `/api/*` route (and everything else except the SPA shell paths) must
//! carry a valid `x-sig-timestamp` + `x-sig` pair over the shared token:
//! missing/malformed headers, out-of-window timestamps and wrong signatures
//! are 401; the SAME accepted signature seen twice inside the window is a
//! replay → 409. The SPA shell (`/`, `/static/*`) and `/api/time` are exempt.

mod support;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_core::auth_sig;
use opencoder_store::{LibsqlStore, Store};
use support::signed_req;
use tower::ServiceExt;

const TOKEN: &str = "sekret-token-123";

async fn app() -> axum::Router {
    app_with_web(true).await
}

async fn app_with_web(web: bool) -> axum::Router {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
        client_override: None,
    });
    opencoder_web::build_app(state, Some(TOKEN.into()), web)
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.clone().oneshot(req).await.expect("router must answer");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}))
    };
    (status, body)
}

#[tokio::test]
async fn api_without_signature_is_401() {
    let app = app().await;
    let (status, body) = send(
        &app,
        Request::builder()
            .uri("/api/health")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].is_string(), "rejection must name the reason");
}

#[tokio::test]
async fn api_with_wrong_signature_is_401() {
    let app = app().await;
    let mut req = signed_req("GET", "/api/health", "not-the-token", None);
    // Swap in a syntactically valid but wrong signature.
    let ts = chrono::Utc::now().timestamp_millis().to_string();
    let sig = auth_sig::sign_hex("not-the-token", &format!("GET\n/api/health\n{ts}\nxyz"));
    *req.headers_mut() = Default::default();
    req.headers_mut()
        .insert(auth_sig::TS_HEADER, ts.parse().unwrap());
    req.headers_mut()
        .insert(auth_sig::SIG_HEADER, sig.parse().unwrap());
    let (status, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stale_timestamp_is_401() {
    let app = app().await;
    let stale = chrono::Utc::now().timestamp_millis() - (auth_sig::REPLAY_WINDOW_MS + 60_000);
    let canon = auth_sig::canonical("GET", "/api/health", stale, b"");
    let sig = auth_sig::sign_hex(TOKEN, &canon);
    let (status, body) = send(
        &app,
        Request::builder()
            .uri("/api/health")
            .header(auth_sig::TS_HEADER, stale.to_string())
            .header(auth_sig::SIG_HEADER, sig)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].as_str().unwrap().contains("window"));
}

#[tokio::test]
async fn fresh_signed_request_is_200() {
    let app = app().await;
    let (status, _) = send(&app, signed_req("GET", "/api/health", TOKEN, None)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn signed_post_body_is_verified() {
    let app = app().await;
    let body = serde_json::json!({ "name": "n1" });
    let (status, _) = send(
        &app,
        signed_req("POST", "/api/nodes/register", TOKEN, Some(body.to_string())),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "signed POST must pass verification");

    // Tampering with the body after signing must fail: sign payload A, send B.
    let tampered = signed_req(
        "POST",
        "/api/nodes/register",
        TOKEN,
        Some(r#"{"name":"n1"}"#.into()),
    );
    let evil = Request::builder()
        .method("POST")
        .uri("/api/nodes/register")
        .header(
            auth_sig::TS_HEADER,
            tampered.headers()[auth_sig::TS_HEADER].clone(),
        )
        .header(
            auth_sig::SIG_HEADER,
            tampered.headers()[auth_sig::SIG_HEADER].clone(),
        )
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"n2"}"#))
        .unwrap();
    let (status, _) = send(&app, evil).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "body swap must not verify"
    );
}

#[tokio::test]
async fn window_edge_timestamp_replay_is_rejected() {
    let app = app().await;
    // F6: a timestamp exactly one replay-window old is still legal — verify's
    // window is INCLUSIVE (`|now-ts| > REPLAY_WINDOW_MS` rejects). The replay
    // cache must keep the signature through that boundary so the second,
    // identical request is a 409, not a free second pass. A literal
    // `now - REPLAY_WINDOW_MS` would race the server clock (it advances
    // between this read and the middleware's own read → 401), so a 2 s slack
    // keeps both requests inside the verify window and lets the replay guard
    // do the rejecting; the exact last-millisecond boundary is pinned by
    // `window_edge_entry_survives_prune_so_replay_is_caught` in `auth_sig_mw`.
    let ts = chrono::Utc::now().timestamp_millis() - auth_sig::REPLAY_WINDOW_MS + 2_000;
    let canon = auth_sig::canonical("GET", "/api/health", ts, b"");
    let sig = auth_sig::sign_hex(TOKEN, &canon);
    // Body isn't Clone, so build two byte-identical requests from one
    // signature pair (same ts + same canonical input → same signature).
    let build = || {
        Request::builder()
            .uri("/api/health")
            .header(auth_sig::TS_HEADER, ts.to_string())
            .header(auth_sig::SIG_HEADER, sig.clone())
            .body(Body::empty())
            .unwrap()
    };
    let (status, body) = send(&app, build()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "inclusive window-edge timestamp must verify: {body}"
    );
    let (status, body) = send(&app, build()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn replayed_signature_is_409() {
    let app = app().await;
    // Body isn't Clone, so build two byte-identical requests from one
    // signature pair (same ts + same canonical input → same signature).
    let (_, ts, _, sig) = support::sig_headers(TOKEN, "GET", "/api/health", b"");
    let build = || {
        Request::builder()
            .uri("/api/health")
            .header(auth_sig::TS_HEADER, ts.clone())
            .header(auth_sig::SIG_HEADER, sig.clone())
            .body(Body::empty())
            .unwrap()
    };
    let (status, _) = send(&app, build()).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(&app, build()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn time_endpoint_is_unsigned() {
    let app = app().await;
    let (status, body) = send(
        &app,
        Request::builder()
            .uri("/api/time")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let now = chrono::Utc::now().timestamp_millis();
    let server = body["server_time_ms"].as_i64().expect("server_time_ms");
    assert!(
        (server - now).abs() < 60_000,
        "server clock must be plausible: {server} vs {now}"
    );
}

#[tokio::test]
async fn shell_paths_are_exempt_but_api_is_not() {
    let app = app_with_web(true).await;
    let (status, _) = send(
        &app,
        Request::builder().uri("/").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "SPA shell must load without a token"
    );
    let (status, _) = send(
        &app,
        Request::builder()
            .uri("/static/app.js")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "embedded console asset: exempt path must be served"
    );
    let (status, _) = send(
        &app,
        Request::builder()
            .uri("/static/nope.js")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "exempt + whitelisted asset; non-whitelisted names still 404"
    );
}

#[tokio::test]
async fn malformed_timestamp_is_401() {
    let app = app().await;
    let (status, _) = send(
        &app,
        Request::builder()
            .uri("/api/health")
            .header(auth_sig::TS_HEADER, "not-a-number")
            .header(auth_sig::SIG_HEADER, "00")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn oversized_body_is_413() {
    let app = app().await;
    let big = "x".repeat(2 * 1024 * 1024 + 1);
    let (status, _) = send(
        &app,
        signed_req(
            "POST",
            "/api/nodes/register",
            TOKEN,
            Some(format!(r#"{{"name":"{big}"}}"#)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}
