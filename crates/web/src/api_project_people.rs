//! Minimal public people directory, without roles or credential metadata.
use crate::{api_project_util::error_500, AppState};
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::Arc;

pub async fn list(State(state): State<Arc<AppState>>) -> Response {
    match state.store.list_users().await {
        Ok(users) => Json(json!({"people": users.into_iter().map(|user| json!({"name":user.name})).collect::<Vec<_>>()})).into_response(),
        Err(error) => error_500(error.to_string()),
    }
}

pub fn write_error(error: anyhow::Error) -> Response {
    if error.is::<opencoder_store::project::people::InvalidProjectUser>() {
        crate::api_project_util::error_400(error.to_string())
    } else {
        error_500(error.to_string())
    }
}
