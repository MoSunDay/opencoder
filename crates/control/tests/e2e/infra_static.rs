//! Infra surface: SPA shell + static assets (unauthenticated), server time,
//! health (auth matrix), readiness and the 404 fallback.

use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

#[tokio::test]
async fn spa_shell_and_static_assets_are_public() {
    let h = Harness::new().await;
    // Unauthenticated bootstrap: the SPA shell and known static assets.
    for (path, marker) in [
        ("/", "<html"),
        ("/static/app.js", "fn"),
        ("/static/app.css", "{"),
    ] {
        let resp = h.req_raw(Method::GET, path, None, None).await;
        assert_eq!(resp.status(), 200, "{path}");
        let body = resp.text().await.unwrap();
        assert!(body.contains(marker), "{path} missing {marker:?}");
    }
    // Unknown asset names are rejected even without auth.
    for path in ["/static/nope.js", "/static/index.html"] {
        let resp = h.req_raw(Method::GET, path, None, None).await;
        assert_eq!(resp.status(), 404, "{path}");
    }
}

#[tokio::test]
async fn time_and_health_auth_matrix() {
    let h = Harness::new().await;
    // /api/time is the unauthenticated readiness probe.
    let resp = h.req_raw(Method::GET, "/api/time", None, None).await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["server_time_ms"].as_i64().unwrap_or(0) > 0, "{body}");

    // /api/health requires the bearer and reports the control role.
    for token in [None, Some("wrong-token")] {
        let resp = h.req_raw(Method::GET, "/api/health", None, token).await;
        assert_eq!(resp.status(), 401);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["ok"], json!(false));
    }
    let (status, body) = h.req(Method::GET, "/api/health", None).await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["role"], json!("control"));
    assert_eq!(
        body["protocol_version"],
        json!(opencoder_core::fleet::PROTOCOL_VERSION)
    );
    assert!(body["commit"].as_str().is_some_and(|c| !c.is_empty()));
    // Bearer token must not be the empty string.
    let resp = h.req_raw(Method::GET, "/api/health", None, Some("")).await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn ready_reports_open_with_online_node() {
    let h = Harness::new().await;
    let (status, body) = h.req(Method::GET, "/api/ready", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["mode"], json!("open"));
    assert_eq!(body["online_nodes"], json!(1));
    assert_eq!(body["ready_nodes"], json!(1));
    assert_eq!(body["control_drained"], json!(false));
}

#[tokio::test]
async fn unknown_api_route_is_404_and_auth_still_applies() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(Method::GET, "/api/definitely-not-a-route", None)
        .await;
    assert_eq!(status, 404, "{body}");
    let resp = h
        .req_raw(Method::GET, "/api/definitely-not-a-route", None, None)
        .await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn favicon_is_auth_exempt() {
    let h = Harness::new().await;
    // There is no dedicated favicon asset: the session-relay fallback answers
    // 404 — the contract under test is that /favicon.ico is on the auth
    // exemption list, so an unauthenticated call must never see 401.
    let (status, body) = h
        .req_bytes(Method::GET, "/favicon.ico", None, None, None, &[])
        .await;
    assert_ne!(status.as_u16(), 401, "{body:?}");
    assert_eq!(status.as_u16(), 404, "{body:?}");
}

#[tokio::test]
async fn non_bearer_authorization_scheme_is_rejected() {
    let h = Harness::new().await;
    // Valid-looking Basic credentials still fail: only the Bearer scheme is
    // accepted (auth_mw::bearer_token).
    let (status, body) = h
        .req_bytes(
            Method::GET,
            "/api/health",
            None,
            None,
            None,
            &[("authorization", "Basic dXNlcjpwYXNz")],
        )
        .await;
    assert_eq!(status.as_u16(), 401, "{body:?}");
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    assert_eq!(parsed["ok"], json!(false));
}

#[tokio::test]
async fn download_service_worker_ships_sw_headers_unauthenticated() {
    let h = Harness::new().await;
    // download-sw.js is a fixed SPA dist whitelist entry and auth-exempt like
    // every /static/ asset; the service-worker contract needs the scope and
    // no-store cache headers.
    let resp = h
        .req_raw(Method::GET, "/static/download-sw.js", None, None)
        .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("service-worker-allowed")
            .and_then(|value| value.to_str().ok()),
        Some("/")
    );
    assert_eq!(
        resp.headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert!(resp
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("javascript")));
    assert!(!resp.bytes().await.unwrap().is_empty());
}

#[tokio::test]
async fn server_time_advances_monotonically() {
    let h = Harness::new().await;
    let read = || async {
        let resp = h.req_raw(Method::GET, "/api/time", None, None).await;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        body["server_time_ms"].as_i64().unwrap_or(0)
    };
    let first = read().await;
    let second = read().await;
    assert!(first > 0, "first read must be a real timestamp");
    assert!(second >= first, "time must never run backwards");
}

#[tokio::test]
async fn ready_fails_while_zero_nodes_are_ready() {
    let h = Harness::new().await;
    // Open mode, healthy online node, but readiness=false: /api/ready must
    // answer 503 with the derived counts. Snapshot propagation is triggered
    // deterministically by a non-gated maintenance read (the fleet client
    // uplinks a fresh Snapshot frame before every operation reply).
    h.node.set_snapshot_opts(None, Some(false));
    h.node
        .set_maintenance("status", 200, json!({"node": {"id": "node-e2e"}}));
    let (status, _) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/maintenance",
            Some(json!({"action": "status"})),
        )
        .await;
    assert_eq!(status, 200);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let body = loop {
        let (status, body) = h.req(Method::GET, "/api/ready", None).await;
        if status.as_u16() == 503 && body["ready_nodes"] == json!(0) {
            break body;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timeout waiting for zero-ready 503: status={status} body={body}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    assert_eq!(body["mode"], json!("open"));
    assert_eq!(body["online_nodes"], json!(1));
    assert_eq!(body["control_drained"], json!(false));
}
