use super::{failure, require_admin};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Extension, Json,
};
use opencoder_core::identity::{token_hash, Identity, Role};
use opencoder_store::AccessToken;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub async fn list(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<Identity>>,
) -> Response {
    if let Some(denied) = require_admin(&identity) {
        return denied;
    }
    match state.store.list_access_tokens().await {
        Ok(tokens) => Json(json!({"tokens":tokens})).into_response(),
        Err(error) => failure(500, error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueToken {
    pub user_name: String,
    pub name: String,
    pub expires_at: Option<i64>,
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<Identity>>,
    Json(body): Json<IssueToken>,
) -> Response {
    if let Some(denied) = require_admin(&identity) {
        return denied;
    }
    if body.user_name == "admin" {
        return failure(400, "startup admin token is read-only");
    }
    let now = chrono::Utc::now().timestamp_millis();
    let name = body.name.trim();
    if name.is_empty()
        || name.len() > 128
        || name.chars().any(char::is_control)
        || body.expires_at.is_some_and(|expiry| expiry <= now)
    {
        return failure(
            400,
            "token name must be 1-128 bytes and expiration must be in the future",
        );
    }
    match state.store.find_user_by_name(&body.user_name).await {
        Ok(Some(user)) if user.role == Role::Admin => {
            return failure(400, "startup admin token is read-only")
        }
        Ok(Some(_)) => {}
        Ok(None) => return failure(404, "user not found"),
        Err(error) => return failure(500, error),
    }
    let plaintext = format!("oc_{}{}", ulid::Ulid::new(), ulid::Ulid::new());
    let token = AccessToken {
        id: ulid::Ulid::new().to_string(),
        user_name: body.user_name,
        name: name.into(),
        created_at: now,
        expires_at: body.expires_at,
        revoked_at: None,
    };
    match state
        .store
        .create_access_token(&token, &token_hash(&plaintext))
        .await
    {
        Ok(()) => Json(json!({"token":plaintext,"metadata":token})).into_response(),
        Err(error) => failure(500, error),
    }
}

pub async fn revoke(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<Identity>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = require_admin(&identity) {
        return denied;
    }
    match state
        .store
        .revoke_access_token(&id, chrono::Utc::now().timestamp_millis())
        .await
    {
        Ok(true) => Json(json!({"ok":true})).into_response(),
        Ok(false) => failure(404, "token not found or startup token is read-only"),
        Err(error) => failure(500, error),
    }
}
