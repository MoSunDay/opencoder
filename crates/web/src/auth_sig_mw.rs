//! Signature middleware — the single auth gate for every route.
//!
//! Replaces the former bearer-token gate. Every request (browser SPA and
//! worker node alike) must carry `x-sig-timestamp` + `x-sig` over the shared
//! token; see `opencoder_core::auth_sig` for the wire format. Exempt paths:
//! `/` and `/static/*` (the SPA shell itself must be loadable before the user
//! can enter the token) and `/api/time` (clock-sync bootstrap).
//!
//! Replay defense is a process-local cache keyed by signature hex: the same
//! valid signature seen twice inside the timestamp window is a replay → 409.
//! Single-instance semantics, cleared on restart — accepted trade-off for v1.

use std::collections::HashMap;
use std::sync::Mutex;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use opencoder_core::auth_sig;

/// Bodies are buffered to compute the signed hash; larger requests are
/// rejected before any verification work.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Upper bound on live replay-cache entries. Pruning normally keeps this far
/// below the cap (5-min TTL); the hard cap is a memory safety net.
const REPLAY_CACHE_CAPACITY: usize = 50_000;

/// Shared verifier state: the token doubles as the HMAC key.
pub struct SigState {
    secret: String,
    seen: Mutex<HashMap<String, i64>>,
}

impl SigState {
    pub fn new(secret: String) -> Self {
        SigState {
            secret,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// True when this exact signature was never seen inside its validity
    /// window. Inserts the signature on success; prunes expired entries first
    /// (bounded work: a full scan only once per request, O(n) with tiny n).
    fn check_and_record(&self, sig: &str, ts_ms: i64, now_ms: i64) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let expiry = ts_ms + auth_sig::REPLAY_WINDOW_MS;
        seen.retain(|_, exp| *exp > now_ms);
        if seen.contains_key(sig) {
            return false;
        }
        if seen.len() >= REPLAY_CACHE_CAPACITY {
            // Safety net against a flood of unique signatures: drop one
            // arbitrary entry. Correctness (replay detection) is window-wide,
            // so evicting early only weakens the cache, never correctness of
            // accepted requests.
            if let Some(k) = seen.keys().next().cloned() {
                seen.remove(&k);
            }
        }
        seen.insert(sig.to_string(), expiry);
        true
    }
}

/// `GET /api/time` — unsigned clock bootstrap so browser clients can compute
/// their offset and stay inside the signature window.
pub async fn server_time() -> impl IntoResponse {
    Json(json!({ "server_time_ms": chrono::Utc::now().timestamp_millis() }))
}

/// Paths that must be reachable WITHOUT a signature.
fn exempt(path: &str) -> bool {
    path == "/" || path.starts_with("/static/") || path == "/api/time" || path == "/favicon.ico"
}

/// The middleware itself. `token = None` disables auth entirely (tests).
pub async fn require_sig(
    State(state): State<Option<std::sync::Arc<SigState>>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(sig) = state.as_ref() else {
        return next.run(req).await;
    };
    if exempt(req.uri().path()) {
        return next.run(req).await;
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({ "error": "body too large to verify" })),
            )
                .into_response()
        }
    };
    let Some(ts_raw) = parts
        .headers
        .get(auth_sig::TS_HEADER)
        .and_then(|v| v.to_str().ok())
    else {
        return deny(StatusCode::UNAUTHORIZED, "missing x-sig-timestamp");
    };
    let Some(sig_raw) = parts
        .headers
        .get(auth_sig::SIG_HEADER)
        .and_then(|v| v.to_str().ok())
    else {
        return deny(StatusCode::UNAUTHORIZED, "missing x-sig");
    };
    let Ok(ts_ms) = ts_raw.trim().parse::<i64>() else {
        return deny(StatusCode::UNAUTHORIZED, "malformed x-sig-timestamp");
    };
    let pq = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| parts.uri.path().to_string());
    let verdict = auth_sig::verify(
        &sig.secret,
        parts.method.as_str(),
        &pq,
        ts_ms,
        now_ms,
        &bytes,
        sig_raw,
    );
    if let Err(e) = verdict {
        let why = match e {
            auth_sig::SigError::TimestampOutOfRange => "timestamp outside 5-minute window",
            auth_sig::SigError::Mismatch => "signature mismatch",
        };
        return deny(StatusCode::UNAUTHORIZED, why);
    }
    if !sig.check_and_record(sig_raw.trim(), ts_ms, now_ms) {
        return deny(StatusCode::CONFLICT, "replayed signature");
    }
    let req = Request::from_parts(parts, Body::from(bytes));
    next.run(req).await
}

fn deny(status: StatusCode, why: &str) -> Response {
    (status, Json(json!({ "error": why }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> SigState {
        SigState::new("s".into())
    }

    #[test]
    fn first_use_is_fresh_second_is_replay() {
        let s = state();
        assert!(s.check_and_record("a", 1_000, 1_500));
        assert!(!s.check_and_record("a", 1_000, 1_600));
        assert!(s.check_and_record("b", 1_000, 1_600));
    }

    #[test]
    fn expiry_prunes_and_frees_the_signature() {
        let s = state();
        assert!(s.check_and_record("a", 0, 0));
        // Window fully elapsed: entry pruned, so re-use is accepted again.
        assert!(s.check_and_record(
            "a",
            2 * auth_sig::REPLAY_WINDOW_MS,
            2 * auth_sig::REPLAY_WINDOW_MS
        ));
    }

    #[test]
    fn exempt_paths_cover_shell_and_clock_bootstrap() {
        assert!(exempt("/"));
        assert!(exempt("/static/app.js"));
        assert!(exempt("/api/time"));
        assert!(!exempt("/api/nodes"));
        assert!(!exempt("/static"));
        assert!(!exempt("/api"));
    }
}
