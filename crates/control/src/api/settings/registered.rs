//! Versioned private profiles and executable registrations, pinned at dispatch.
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_core::fleet::RpcReply;
use opencoder_core::harness::{CodexSettings, RunnerSettings, RuntimeSettings};
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn snapshot(state: &AppState) -> anyhow::Result<Option<Box<RuntimeSettings>>> {
    let mut settings = RuntimeSettings::default();
    for value in state.fleet.definitions("codex_profile").await? {
        let name = value["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("profile name missing"))?;
        settings
            .profiles
            .insert(name.into(), serde_json::from_value(value)?);
    }
    for value in state.fleet.definitions("runner").await? {
        let name = value["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("runner name missing"))?;
        settings
            .runners
            .insert(name.into(), serde_json::from_value(value)?);
    }
    anyhow::ensure!(
        serde_json::to_vec(&settings)?.len() <= 768 * 1024,
        "Registered execution settings exceed dispatch frame budget"
    );
    Ok(Some(Box::new(settings)))
}

async fn list(state: &AppState, namespace: &str) -> Response {
    match state.fleet.definitions(namespace).await {
        Ok(values) => super::super::response(RpcReply::ok(json!({"items":values}))),
        Err(error) => super::super::error_500(error.to_string()),
    }
}

async fn save(state: &AppState, namespace: &str, name: &str, settings: Value) -> Response {
    if let Err(error) = opencoder_core::agent::validate_agent_name(name) {
        return super::super::error_400(error);
    }
    let _gate = state.placement.lock().await;
    let result = async {
        let old = state.fleet.definition(namespace, name).await?;
        let revision = old
            .and_then(|v| v["revision"].as_u64())
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("configuration revision exhausted"))?;
        let value = json!({"name":name,"revision":revision,"settings":settings});
        state.fleet.put_definition(namespace, name, &value).await?;
        Ok::<_, anyhow::Error>(value)
    }
    .await;
    match result {
        Ok(value) => super::super::response(RpcReply::ok(value)),
        Err(error) => super::super::error_500(error.to_string()),
    }
}

pub async fn profiles(State(state): State<Arc<AppState>>) -> Response {
    list(&state, "codex_profile").await
}
pub async fn save_profile(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(settings): Json<CodexSettings>,
) -> Response {
    if let Err(error) = settings.validate() {
        return super::super::error_400(error);
    }
    save(&state, "codex_profile", &name, json!(settings)).await
}
pub async fn runners(State(state): State<Arc<AppState>>) -> Response {
    list(&state, "runner").await
}
pub async fn save_runner(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(settings): Json<RunnerSettings>,
) -> Response {
    if let Err(error) = settings.validate() {
        return super::super::error_400(error);
    }
    save(&state, "runner", &name, json!(settings)).await
}
