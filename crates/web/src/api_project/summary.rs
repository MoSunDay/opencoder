//! Summary history and restoration share project read/write permissions.
use crate::{
    api_project_util::{error_400, error_404, error_500, require_deps},
    AppState,
};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use opencoder_store::project::summary::{SummaryScope, SUMMARY_HISTORY_LIMIT};
use serde_json::json;
use std::sync::Arc;

fn scope(collection: &str) -> Option<SummaryScope> {
    match collection {
        "goals" => Some(SummaryScope::Project),
        "initiatives" => Some(SummaryScope::Initiative),
        _ => None,
    }
}

pub async fn versions(
    State(state): State<Arc<AppState>>,
    Path((collection, id)): Path<(String, String)>,
) -> Response {
    let Some(scope) = scope(&collection) else {
        return error_404("summary scope not found");
    };
    let deps = match require_deps(&state) {
        Ok(deps) => deps,
        Err(response) => return *response,
    };
    match deps.projects.summary_versions(scope, &id).await {
        Ok(Some(versions)) => {
            Json(json!({"versions":versions,"limit":SUMMARY_HISTORY_LIMIT})).into_response()
        }
        Ok(None) => error_404("project or initiative not found"),
        Err(error) => error_500(error.to_string()),
    }
}

pub async fn restore(
    State(state): State<Arc<AppState>>,
    Path((collection, id, version)): Path<(String, String, i64)>,
) -> Response {
    let Some(scope) = scope(&collection) else {
        return error_404("summary scope not found");
    };
    if version <= 0 {
        return error_400("summary version must be positive");
    }
    let deps = match require_deps(&state) {
        Ok(deps) => deps,
        Err(response) => return *response,
    };
    match deps
        .projects
        .restore_summary(scope, &id, version, opencoder_core::message::now_ms())
        .await
    {
        Ok(Some(version)) => Json(version).into_response(),
        Ok(None) => error_404("概述版本不存在或已超出最近 10 个版本"),
        Err(error) => error_500(error.to_string()),
    }
}
