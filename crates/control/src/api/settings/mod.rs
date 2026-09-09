//! Private Harness configuration. NFS carries Agent references, never these env values.
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_core::{
    fleet::{ExecutionCommand, NodeOperation, NodeScheduling, RpcReply},
    harness::CodexSettings,
};
use serde_json::json;
use std::sync::Arc;

pub async fn codex(state: &AppState) -> anyhow::Result<Option<Box<CodexSettings>>> {
    state
        .fleet
        .definition("harness", "codex")
        .await?
        .map(|value| serde_json::from_value(value["settings"].clone()).map_err(Into::into))
        .transpose()
}

pub async fn get_harnesses(State(state): State<Arc<AppState>>) -> Response {
    match state.fleet.definition("harness", "codex").await {
        Ok(value) => super::response(RpcReply::ok(json!({"harnesses": [
            {"name":"opencoder", "managed":false},
            {"name":"codex", "managed":value.is_some(), "revision":value.as_ref().map(|v| &v["revision"]),
             "settings":value.map(|v| v["settings"].clone()).unwrap_or(json!(CodexSettings::default()))}
        ]}))),
        Err(error) => super::error_500(error.to_string()),
    }
}

pub async fn save_harness(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(settings): Json<CodexSettings>,
) -> Response {
    if name != "codex" {
        return super::error_400("only Codex settings are supported".into());
    }
    if let Err(error) = settings.validate() {
        return super::error_400(error);
    }
    let _gate = state.placement.lock().await;
    let result = async {
        let old = state.fleet.definition("harness", "codex").await?;
        let revision = old
            .and_then(|v| v["revision"].as_u64())
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Harness revision exhausted"))?;
        let value = json!({"settings":settings,"revision":revision});
        state
            .fleet
            .put_definition("harness", "codex", &value)
            .await?;
        Ok::<_, anyhow::Error>(value)
    }
    .await;
    match result {
        Ok(value) => super::response(RpcReply::ok(value)),
        Err(error) => super::error_500(error.to_string()),
    }
}

pub async fn save_scheduling(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(settings): Json<NodeScheduling>,
) -> Response {
    if let Err(error) = settings.validate() {
        return super::error_400(error);
    }
    super::response(
        state
            .hub
            .call(
                &id,
                NodeOperation::Maintenance {
                    command: ExecutionCommand {
                        action: "configure_scheduling".into(),
                        input: json!(settings),
                    },
                },
            )
            .await,
    )
}
