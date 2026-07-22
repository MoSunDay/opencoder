//! Bearer-token authentication middleware. Applied to every route (the HTML
//! manager at `/` and all `/api/*`). A request passes if EITHER:
//!   * its `Authorization: Bearer <T>` header matches the configured token, OR
//!   * its query string contains `token=<T>` (required for the browser
//!     `EventSource` API, which cannot set request headers).
//!     Otherwise it is rejected with `401 Unauthorized`.

use axum::extract::State;
use axum::http::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// The middleware state is the shared bearer token.
pub type TokenState = String;

/// Extract the `token=` value from a raw query string. The token is an ULID
/// (`0-9A-Z`), so no percent-decoding is needed for the supported format; we
/// still handle `%XX` for robustness against a client that encodes.
fn query_token(q: &str) -> Option<String> {
    for pair in q.split('&') {
        let mut it = pair.splitn(2, '=');
        if it.next() == Some("token") {
            return it.next().map(percent_decode);
        }
    }
    None
}

/// Minimal percent-decoding for ASCII. Sufficient for a ULID token.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Reject unmatched requests with a JSON 401.
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "ok": false, "error": "unauthorized: missing or invalid token" })),
    )
        .into_response()
}

pub async fn require_token(
    State(token): State<TokenState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // 1. Authorization: Bearer <T>
    if let Some(h) = req.headers().get(axum::http::header::AUTHORIZATION) {
        if let Ok(v) = h.to_str() {
            if v.trim() == format!("Bearer {token}") {
                return next.run(req).await;
            }
            // also accept a bare token (lenient)
            if v.trim() == token {
                return next.run(req).await;
            }
        }
    }
    // 2. ?token=<T> (EventSource cannot set headers)
    if let Some(q) = req.uri().query() {
        if let Some(t) = query_token(q) {
            if t == token {
                return next.run(req).await;
            }
        }
    }
    unauthorized()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_token_finds_pair() {
        assert_eq!(query_token("token=ABC123"), Some("ABC123".into()));
        assert_eq!(query_token("after=3&token=XYZ"), Some("XYZ".into()));
        assert_eq!(query_token("token=AB&x=1"), Some("AB".into()));
    }

    #[test]
    fn query_token_absent() {
        assert_eq!(query_token("after=3"), None);
        assert_eq!(query_token(""), None);
    }

    #[test]
    fn percent_decode_handles_hex() {
        assert_eq!(percent_decode("foo%2Bbar"), "foo+bar");
        assert_eq!(percent_decode("plain"), "plain");
    }

    #[test]
    fn hex_parses() {
        assert_eq!(hex(b'A'), Some(10));
        assert_eq!(hex(b'9'), Some(9));
        assert_eq!(hex(b'g'), None);
    }
}
