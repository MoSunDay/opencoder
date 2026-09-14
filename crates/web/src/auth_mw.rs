//! Bearer-token middleware shared by the control and compatibility servers.
//!
//! Two token families authenticate, both resolving to an [`Identity`] injected
//! into the request extensions:
//!
//! 1. the seed token passed at startup — constant-time compared, maps to the
//!    bootstrap `admin` identity (keeps node machine channels and `ctl` working);
//! 2. platform users created via the admin API — looked up by sha256 digest;
//!    only digests are ever stored or queried.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use opencoder_core::identity::{token_hash, Identity};
use opencoder_store::Store;
use serde_json::json;
use std::sync::Arc;

/// Shared verifier state. Neither token nor digests are returned or logged.
pub struct AuthState {
    seed_token: Option<String>,
    lookup: Arc<dyn Store>,
}

impl AuthState {
    pub fn new(seed_token: String, lookup: Arc<dyn Store>) -> Self {
        Self {
            seed_token: Some(seed_token),
            lookup,
        }
    }
}

/// `GET /api/time` remains an unauthenticated compatibility/readiness endpoint.
pub async fn server_time() -> impl IntoResponse {
    Json(json!({ "server_time_ms": chrono::Utc::now().timestamp_millis() }))
}

/// Unauthenticated paths (shell assets, readiness). Also honored by the
/// control-plane role gate.
pub fn exempt(path: &str) -> bool {
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

/// Resolve a bearer credential to an identity, without logging secrets.
/// Owns its inputs so the returned future is `Send` under any caller.
async fn identify(seed: Option<String>, lookup: Arc<dyn Store>, token: String) -> Option<Identity> {
    if seed
        .as_deref()
        .is_some_and(|expected| token_eq(expected, &token))
    {
        return Some(Identity::admin("admin"));
    }
    let user = lookup
        .find_user_by_token_hash(&token_hash(&token))
        .await
        .ok()??;
    Some(Identity {
        name: user.name,
        role: user.role,
    })
}

pub async fn require_bearer(
    State(state): State<Option<Arc<AuthState>>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let Some(auth) = state.clone() else {
        return next.run(req).await;
    };
    if exempt(req.uri().path()) {
        return next.run(req).await;
    }
    let token = bearer_token(&req).map(str::to_owned);
    let identity = match token {
        None => None,
        Some(token) => identify(auth.seed_token.clone(), auth.lookup.clone(), token).await,
    };
    match identity {
        Some(identity) => {
            req.extensions_mut().insert(identity);
            next.run(req).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "ok": false, "error": "invalid bearer token" })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::identity::Role;

    fn request(value: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri("/api/health");
        if let Some(value) = value {
            builder = builder.header(header::AUTHORIZATION, value);
        }
        builder.body(Body::empty()).unwrap()
    }

    async fn state_with(user: Option<(&str, &str, Role)>) -> AuthState {
        let store = opencoder_store::LibsqlStore::open_memory().await.unwrap();
        if let Some((name, token, role)) = user {
            store
                .create_user(name, &token_hash(token), role, 1)
                .await
                .unwrap();
        }
        AuthState::new("seed-secret".into(), std::sync::Arc::new(store))
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

    #[tokio::test]
    async fn seed_token_maps_to_bootstrap_admin() {
        let state = state_with(None).await;
        let identity = identify(
            state.seed_token.clone(),
            state.lookup.clone(),
            "seed-secret".into(),
        )
        .await;
        assert_eq!(identity, Some(Identity::admin("admin")));
        assert_eq!(identity.map(|i| i.role.as_str()), Some("admin"));
    }

    #[tokio::test]
    async fn user_tokens_map_to_their_role() {
        let state = state_with(Some(("alice", "alice-token", Role::User))).await;
        let identify =
            |token: &str| identify(state.seed_token.clone(), state.lookup.clone(), token.into());
        let identity = identify("alice-token").await;
        assert_eq!(
            identity.map(|i| (i.name, i.role.as_str())),
            Some(("alice".to_string(), "user"))
        );
        // Seed and unknown tokens never cross over.
        assert!(identify("seed-secret").await.is_some());
        assert!(identify("nope").await.is_none());
    }
}
