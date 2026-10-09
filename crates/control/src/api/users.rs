//! Administrator-managed identities. Startup admin is protected; token issuance
//! is a separate operation so each user can own several credentials.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use opencoder_core::identity::{parse_role, Identity, Role};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub mod tokens;

pub async fn me(identity: Option<Extension<Identity>>) -> Response {
    let identity = identity
        .map(|Extension(i)| i)
        .unwrap_or_else(|| Identity::admin("admin"));
    Json(json!({"name":identity.name,"role":identity.role.as_str()})).into_response()
}

pub(super) fn require_admin(identity: &Option<Extension<Identity>>) -> Option<Response> {
    identity
        .as_ref()
        .is_some_and(|Extension(i)| !i.is_admin())
        .then(|| failure(403, "admin role required"))
}

pub(super) fn failure(status: u16, message: impl ToString) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(json!({"ok":false,"error":message.to_string()})),
    )
        .into_response()
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<Identity>>,
) -> Response {
    if let Some(denied) = require_admin(&identity) {
        return denied;
    }
    match state.store.list_users().await {
        Ok(users) => Json(json!({"users":users})).into_response(),
        Err(error) => failure(500, error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateUser {
    pub name: String,
    pub role: String,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn managed_role(value: &str) -> Option<Role> {
    parse_role(value).filter(|role| *role != Role::Admin)
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<Identity>>,
    Json(body): Json<CreateUser>,
) -> Response {
    if let Some(denied) = require_admin(&identity) {
        return denied;
    }
    let name = body.name.trim();
    if !valid_name(name) || name == "admin" {
        return failure(
            400,
            "name must be 1-64 bytes without whitespace; admin is reserved",
        );
    }
    let Some(role) = managed_role(&body.role) else {
        return failure(
            400,
            "role must be editor or viewer; admin uses the startup token",
        );
    };
    match state
        .store
        .create_user(name, "", role, chrono::Utc::now().timestamp_millis())
        .await
    {
        Ok(user) => Json(json!({"user":user})).into_response(),
        Err(error) => {
            let status = if matches!(state.store.find_user_by_name(name).await, Ok(Some(_))) {
                409
            } else {
                500
            };
            failure(status, error)
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateUser {
    pub role: String,
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<Identity>>,
    Path(name): Path<String>,
    Json(body): Json<UpdateUser>,
) -> Response {
    if let Some(denied) = require_admin(&identity) {
        return denied;
    }
    let Some(role) = managed_role(&body.role) else {
        return failure(400, "role must be editor or viewer");
    };
    if name == "admin" {
        return failure(400, "startup admin is read-only");
    }
    match state.store.find_user_by_name(&name).await {
        Ok(Some(user)) if user.role == Role::Admin || name == "admin" => {
            return failure(400, "startup admin is read-only")
        }
        Ok(None) => return failure(404, "user not found"),
        Err(error) => return failure(500, error),
        _ => {}
    }
    match state.store.update_user_role(&name, role).await {
        Ok(true) => Json(json!({"ok":true})).into_response(),
        Ok(false) => failure(409, "user changed; refresh and retry"),
        Err(error) => failure(500, error),
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
    if name == "admin" {
        return failure(400, "startup admin is read-only");
    }
    match state.store.find_user_by_name(&name).await {
        Ok(Some(user)) if user.role == Role::Admin => {
            return failure(400, "startup admin is read-only")
        }
        Err(error) => return failure(500, error),
        _ => {}
    }
    match state.store.delete_user(&name).await {
        Ok(true) => Json(json!({"ok":true})).into_response(),
        Ok(false) => failure(404, "user not found"),
        Err(error) => failure(500, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_roles_are_explicit() {
        assert!(valid_name("alice"));
        for value in ["", "a b", "a\nb"] {
            assert!(!valid_name(value));
        }
        assert!(!valid_name(&"x".repeat(65)));
        assert_eq!(managed_role("editor"), Some(Role::Editor));
        assert_eq!(managed_role("viewer"), Some(Role::Viewer));
        for value in ["admin", "root", "user"] {
            assert_eq!(managed_role(value), None);
        }
    }
}
