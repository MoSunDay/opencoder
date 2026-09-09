//! Shared Bearer-authenticated request helpers for integration tests.
//!
//! Every test that exercises a token-bearing app (`build_app(.., Some(token),
//! ..)`) must send `Authorization: Bearer <token>`.
//!
//! * [`authed_req`] — `axum` oneshot requests
//! * [`authed_post_json`]/[`authed_get_json`] — live reqwest servers
//! * [`auth_header`] — raw auth header (e.g. streaming GETs)

#![allow(dead_code)] // each test file uses a different subset

use axum::body::Body;
use axum::http::{header, request::Request};

pub fn auth_header(token: &str) -> (header::HeaderName, String) {
    (header::AUTHORIZATION, format!("Bearer {token}"))
}

/// Build an authenticated `axum` oneshot request. `body = Some(json)` implies the
/// JSON content-type; GETs pass `None`.
pub fn authed_req(method: &str, uri: &str, token: &str, body: Option<String>) -> Request<Body> {
    let bytes = body.clone().map(String::into_bytes).unwrap_or_default();
    let (name, value) = auth_header(token);
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(name, value);
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    b.body(Body::from(bytes)).unwrap()
}

/// Authenticate + send one JSON POST against a live server; returns (status, body).
pub async fn authed_post_json(
    base: &str,
    path: &str,
    token: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let bytes = serde_json::to_vec(&body).unwrap();
    let (name, value) = auth_header(token);
    let resp = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header(name, value)
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

/// Authenticate + send one GET against a live server; returns (status, body).
pub async fn authed_get_json(
    base: &str,
    path: &str,
    token: &str,
) -> (reqwest::StatusCode, serde_json::Value) {
    let (name, value) = auth_header(token);
    let resp = reqwest::Client::new()
        .get(format!("{base}{path}"))
        .header(name, value)
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
pub mod project_mock;
