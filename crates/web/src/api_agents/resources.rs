//! Both Web and Control expose the same agent-identity resource contract.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use opencoder_agents::resources::{self, RestoreRequest, SaveRequest};
use serde_json::json;
use std::{io, sync::Arc};

fn response(result: io::Result<resources::ResourceView>) -> Response {
    match result {
        Ok(view) => Json(view).into_response(),
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::NotFound => StatusCode::NOT_FOUND,
                io::ErrorKind::AlreadyExists => StatusCode::CONFLICT,
                io::ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
                io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, Json(json!({"ok":false,"error":error.to_string()}))).into_response()
        }
    }
}

pub async fn get(
    State(_): State<Arc<AppState>>,
    Path((name, cat)): Path<(String, String)>,
) -> Response {
    response(resources::read(&name, &cat))
}

pub async fn put(
    State(_): State<Arc<AppState>>,
    Path((name, cat)): Path<(String, String)>,
    Json(body): Json<SaveRequest>,
) -> Response {
    // Accepted platform tasks keep their pinned resources. New admissions read the new version.
    response(resources::save(&name, &cat, body))
}

pub async fn restore(
    State(_): State<Arc<AppState>>,
    Path((name, cat)): Path<(String, String)>,
    Json(body): Json<RestoreRequest>,
) -> Response {
    response(resources::restore(&name, &cat, body))
}
