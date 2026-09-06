//! Bearer-token middleware shared by the control and compatibility servers.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Shared verifier state. The token is never returned or logged.
pub struct AuthState {
    token: String,
}

impl AuthState {
    pub fn new(token: String) -> Self {
        Self { token }
    }
}

/// `GET /api/time` remains an unauthenticated compatibility/readiness endpoint.
pub async fn server_time() -> impl IntoResponse {
    Json(json!({ "server_time_ms": chrono::Utc::now().timestamp_millis() }))
}

fn exempt(path: &str) -> bool {
    path == "/" || path.starts_with("/static/") || path == "/api/time" || path == "/favicon.ico"
}

fn bearer_token(req: &Request<Body>) -> Option<&str> {
    let value = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, credentials) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = credentials.trim_start_matches(' ');
    (!token.is_empty() && !token.bytes().any(|byte| byte.is_ascii_whitespace())).then_some(token)
}

fn token_eq(expected: &str, actual: &str) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    expected
        .bytes()
        .zip(actual.bytes())
        .fold(0u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

pub async fn require_bearer(
    State(state): State<Option<std::sync::Arc<AuthState>>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(auth) = state.as_ref() else {
        return next.run(req).await;
    };
    if exempt(req.uri().path()) {
        return next.run(req).await;
    }
    if bearer_token(&req).is_some_and(|token| token_eq(&auth.token, token)) {
        return next.run(req).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "ok": false, "error": "invalid bearer token" })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri("/api/health");
        if let Some(value) = value {
            builder = builder.header(header::AUTHORIZATION, value);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn parses_only_nonempty_bearer_credentials() {
        assert_eq!(
            bearer_token(&request(Some("Bearer secret"))),
            Some("secret")
        );
        assert_eq!(
            bearer_token(&request(Some("bearer  secret"))),
            Some("secret")
        );
        assert_eq!(bearer_token(&request(Some("Basic secret"))), None);
        assert_eq!(bearer_token(&request(Some("Bearer "))), None);
        assert_eq!(bearer_token(&request(Some("Bearer secret extra"))), None);
        assert_eq!(bearer_token(&request(None)), None);
    }

    #[test]
    fn equality_checks_the_complete_token() {
        assert!(token_eq("secret", "secret"));
        assert!(!token_eq("secret", "wrong!"));
        assert!(!token_eq("secret", "secret-longer"));
    }

    #[test]
    fn exempt_paths_cover_shell_and_time() {
        assert!(exempt("/"));
        assert!(exempt("/static/app.js"));
        assert!(exempt("/api/time"));
        assert!(!exempt("/api/health"));
    }
}
