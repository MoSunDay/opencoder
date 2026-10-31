//! Trigger endpoint for freshness runs: create the run record through the
//! data plane, then hand it to the background orchestrator.
use crate::{ontology_freshness::orchestrator, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use opencoder_ontology::{AppError, FreshnessOperation, FreshnessRun};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) fn routes() -> Router<Arc<AppState>> {
    Router::new().route(
        "/api/ontology/envs/:env/freshness/policies/:id/runs",
        post(trigger),
    )
}

async fn trigger(
    State(state): State<Arc<AppState>>,
    Path((env, id)): Path<(String, Uuid)>,
) -> Response {
    let result: Result<(Value, Option<FreshnessRun>, Value), AppError> = async {
        let ontology = state.ontology.as_ref().ok_or(AppError::NotFound)?;
        // Fetch the policy first so unknown ids fail before any run exists.
        let policy = ontology
            .freshness(env.clone(), FreshnessOperation::PolicyGet(id))
            .await?["item"]
            .clone();
        let created = ontology
            .freshness(env.clone(), FreshnessOperation::RunCreate { policy_id: id })
            .await?;
        // A run record that cannot be decoded stays queued; the startup
        // reconciler fails its pending items instead of failing the request.
        let run = serde_json::from_value(created["item"].clone())
            .map_err(|error| {
                tracing::warn!(%error, "Freshness run record is undecodable");
                error
            })
            .ok();
        Ok((created["item"].clone(), run, policy))
    }
    .await;
    match result {
        Ok((item, run, policy)) => {
            if let Some(run) = run {
                let driver = state.clone();
                tokio::spawn(orchestrator::execute(driver, env, policy, run));
            }
            (StatusCode::ACCEPTED, Json(json!({ "item": item }))).into_response()
        }
        Err(error) => error.into_response(),
    }
}
