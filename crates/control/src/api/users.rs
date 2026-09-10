//! Platform-user administration (`/api/users`) and identity probe
//! (`/api/me`). Admin-only; the role gate plus explicit handler checks
//! enforce it. Tokens are returned exactly once in the creation response.

use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use opencoder_core::identity::{parse_role, token_hash, Identity};
use opencoder_store::PlatformUser;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// `GET /api/me` — the authenticated caller. Without bearer middleware
/// (auth-disabled deployments) defaults to the bootstrap admin view.
pub async fn me(identity: Option<Extension<Identity>>) -> Response {
    let identity = identity
        .map(|Extension(i)| i)
        .unwrap_or_else(|| Identity::admin("admin"));
    Json(json!({"name": identity.name, "role": identity.role.as_str()})).into_response()
}

/// The caller when the bearer middleware ran; `None` means auth is disabled
/// and the request is treated as the implicit admin.
fn caller(identity: &Option<Extension<Identity>>) -> Option<&Identity> {
    identity.as_ref().map(|Extension(i)| i)
}

fn require_admin(identity: &Option<Extension<Identity>>) -> Option<Response> {
    caller(identity).is_some_and(|i| !i.is_admin()).then(|| {
        (
            StatusCode::FORBIDDEN,
            Json(json!({"ok": false, "error": "admin role required"})),
        )
            .into_response()
    })
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<Identity>>,
) -> Response {
    if let Some(denied) = require_admin(&identity) {
        return denied;
    }
    match state.store.list_users().await {
        Ok(users) => Json(json!({ "users": users })).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": format!("list users: {error:#}")})),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateUser {
    pub name: String,
    pub role: String,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// Wire token = `oc_` + two ULIDs (80 random bits each, 160 total); only
/// the sha256 digest is stored.
fn new_token() -> String {
    format!("oc_{}{}", ulid::Ulid::new(), ulid::Ulid::new())
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<Identity>>,
    Json(request): Json<CreateUser>,
) -> Response {
    if let Some(denied) = require_admin(&identity) {
        return denied;
    }
    let name = request.name.trim();
    if !valid_name(name) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": "name must be 1-64 chars without spaces or control characters"})),
        )
            .into_response();
    }
    let Some(role) = parse_role(request.role.trim()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": "role must be one of admin, root, user"})),
        )
            .into_response();
    };
    let token = new_token();
    let user = state
        .store
        .create_user(name, &token_hash(&token), role, now_ms())
        .await;
    match user {
        Ok(user) => Json(json!({"user": user_view(&user), "token": token})).into_response(),
        Err(error) => {
            let status = match state.store.find_user_by_name(name).await {
                Ok(Some(_)) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(json!({"ok": false, "error": format!("create user: {error:#}")})),
            )
                .into_response()
        }
    }
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<Identity>>,
    Path(name): Path<String>,
) -> Response {
    if let Some(denied) = require_admin(&identity) {
        return denied;
    }
    if caller(&identity).is_some_and(|i| i.name == name) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": "cannot delete the caller's own account"})),
        )
            .into_response();
    }
    // The last-admin guard runs inside the store's delete statement, so two
    // admins racing to delete each other can never both pass and empty the
    // table of admins.
    match state.store.delete_user_guarding_last_admin(&name).await {
        Ok(opencoder_store::GuardedDelete::Deleted) => Json(json!({"ok": true})).into_response(),
        Ok(opencoder_store::GuardedDelete::Missing) => (
            StatusCode::NOT_FOUND,
            Json(json!({"ok": false, "error": "user not found"})),
        )
            .into_response(),
        Ok(opencoder_store::GuardedDelete::LastAdmin) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": "cannot delete the last admin"})),
        )
            .into_response(),
        Err(error) => store_error("delete user", error),
    }
}

fn store_error(what: &str, error: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"ok": false, "error": format!("{what}: {error:#}")})),
    )
        .into_response()
}

fn user_view(user: &PlatformUser) -> serde_json::Value {
    json!({"name": user.name, "role": user.role.as_str(), "created_at": user.created_at})
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_reject_spaces_and_controls() {
        assert!(valid_name("alice"));
        assert!(valid_name("ops-01"));
        assert!(!valid_name(""));
        assert!(!valid_name("a b"));
        assert!(!valid_name("a\nb"));
        assert!(!valid_name(&"x".repeat(65)));
    }

    #[test]
    fn tokens_carry_two_ulids_and_oc_prefix() {
        let token = new_token();
        assert!(token.starts_with("oc_"));
        assert_eq!(token.len(), "oc_".len() + 26 * 2);
        assert_ne!(token, new_token());
    }
}
