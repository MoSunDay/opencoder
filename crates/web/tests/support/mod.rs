//! Shared signed-request helpers for integration tests.
//!
//! The production auth gate is an HMAC signature over
//! `"{METHOD}\n{path_and_query}\n{ts}\n{sha256(body)}"` (see
//! `opencoder_core::auth_sig`). Every test that exercises a token-bearing app
//! (`build_app(.., Some(token), ..)`) must sign its requests with the same
//! token; helpers here cover the three transports in use:
//!
//! * [`signed_req`] — `axum` oneshot requests
//! * [`signed_post_json`]/[`signed_get_json`] — live reqwest servers
//! * [`sig_headers`] — raw header pair (e.g. SSE GETs with response streaming)

#![allow(dead_code)] // each test file uses a different subset

use axum::body::Body;
use axum::http::request::Request;
use opencoder_core::auth_sig;

/// Compute the header pair for one request. `path_and_query` must match the
/// request URI exactly (query string included) — it is part of the signature.
pub fn sig_headers(
    token: &str,
    method: &str,
    path_and_query: &str,
    body: &[u8],
) -> (&'static str, String, &'static str, String) {
    let ts = chrono::Utc::now().timestamp_millis();
    let canon = auth_sig::canonical(method, path_and_query, ts, body);
    (
        auth_sig::TS_HEADER,
        ts.to_string(),
        auth_sig::SIG_HEADER,
        auth_sig::sign_hex(token, &canon),
    )
}

/// Build a signed `axum` oneshot request. `body = Some(json)` implies the
/// JSON content-type; GETs pass `None`.
pub fn signed_req(method: &str, uri: &str, token: &str, body: Option<String>) -> Request<Body> {
    let bytes = body.clone().map(String::into_bytes).unwrap_or_default();
    let (_, ts, _, sig) = sig_headers(token, method, uri, &bytes);
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(auth_sig::TS_HEADER, ts)
        .header(auth_sig::SIG_HEADER, sig);
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    b.body(Body::from(bytes)).unwrap()
}

/// Sign + send one JSON POST against a live server; returns (status, body).
pub async fn signed_post_json(
    base: &str,
    path: &str,
    token: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let bytes = serde_json::to_vec(&body).unwrap();
    let (_, ts, _, sig) = sig_headers(token, "POST", path, &bytes);
    let resp = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header(auth_sig::TS_HEADER, ts)
        .header(auth_sig::SIG_HEADER, sig)
        .header("content-type", "application/json")
        .body(bytes)
        .send()
        .await
        .expect("server must answer");
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v = if text.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
    };
    (status, v)
}

/// Sign + send one GET against a live server; returns (status, body).
pub async fn signed_get_json(
    base: &str,
    path: &str,
    token: &str,
) -> (reqwest::StatusCode, serde_json::Value) {
    let (_, ts, _, sig) = sig_headers(token, "GET", path, b"");
    let resp = reqwest::Client::new()
        .get(format!("{base}{path}"))
        .header(auth_sig::TS_HEADER, ts)
        .header(auth_sig::SIG_HEADER, sig)
        .send()
        .await
        .expect("server must answer");
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v = if text.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
    };
    (status, v)
}
pub mod project_app;
