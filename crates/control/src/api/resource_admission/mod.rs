//! Durable dispatch admission backed by an external resource authority.
//! The request and grant are opaque: platform code does not implement device policy.
use crate::{api, AppState};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use opencoder_core::fleet::{Assignment, ExecutionStatus, RpcReply};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};

const NAMESPACE: &str = "resource_admission_provider";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub endpoint: String,
    pub token_file: PathBuf,
}

async fn provider(state: &AppState) -> anyhow::Result<Provider> {
    let value = state
        .fleet
        .definition(NAMESPACE, "current")
        .await?
        .ok_or_else(|| anyhow::anyhow!("resource admission provider is not configured"))?;
    Ok(serde_json::from_value(value)?)
}

pub async fn save(State(state): State<Arc<AppState>>, Json(value): Json<Provider>) -> Response {
    if !value.token_file.is_absolute()
        || !reqwest::Url::parse(&value.endpoint).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
    {
        return api::error_400("invalid resource provider configuration".into());
    }
    match state
        .fleet
        .put_definition(NAMESPACE, "current", &json!(value))
        .await
    {
        Ok(_) => Json(json!({"enabled":true})).into_response(),
        Err(_) => api::error_500("resource provider configuration unavailable".into()),
    }
}

pub async fn get(State(state): State<Arc<AppState>>) -> Response {
    match state.fleet.definition(NAMESPACE, "current").await {
        Ok(value) => Json(json!({"enabled":value.is_some()})).into_response(),
        Err(_) => api::error_500("resource provider configuration unavailable".into()),
    }
}

async fn call(state: &AppState, body: Value) -> anyhow::Result<Value> {
    let config = provider(state).await?;
    let token = tokio::fs::read_to_string(config.token_file).await?;
    let response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?
        .post(config.endpoint)
        .bearer_auth(token.trim())
        .json(&body)
        .send()
        .await?;
    anyhow::ensure!(
        response.status().is_success(),
        "resource authority unavailable"
    );
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n <= 1024 * 1024),
        "resource response too large"
    );
    let bytes = response.bytes().await?;
    anyhow::ensure!(bytes.len() <= 1024 * 1024, "resource response too large");
    Ok(serde_json::from_slice(&bytes)?)
}

/// Called after the assignment is durable, before any node Create operation.
pub(crate) async fn ready(state: &AppState, assignment: &Assignment) -> anyhow::Result<bool> {
    let Some(request) = assignment.request.input.get("_resource_request") else {
        return Ok(true);
    };
    anyhow::ensure!(request.is_object(), "resource request must be an object");
    let value = call(
        state,
        json!({"action":"acquire", "execution_id":assignment.index.id,
        "request":request}),
    )
    .await?;
    match value["status"].as_str() {
        Some("granted") => Ok(true),
        Some("waiting") => Ok(false),
        _ => anyhow::bail!("invalid resource admission response"),
    }
}

pub async fn inspect(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    invoke(state, id, json!({"action":"inspect"})).await
}

pub async fn command(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<Value>,
) -> Response {
    if !matches!(
        input["action"].as_str(),
        Some("started" | "uncertain" | "release" | "release_batch")
    ) {
        return api::error_400("unsupported resource operation".into());
    }
    invoke(state, id, input).await
}

async fn invoke(state: Arc<AppState>, id: String, mut input: Value) -> Response {
    let assignment = match state.fleet.assignment(&id).await {
        Ok(Some(value)) if value.request.input.get("_resource_request").is_some() => value,
        Ok(_) => return api::error_404("managed resource assignment not found"),
        Err(_) => return api::error_500("resource assignment unavailable".into()),
    };
    if input["action"] == "release_batch" {
        let terminal = state
            .fleet
            .index(&id)
            .await
            .ok()
            .flatten()
            .is_some_and(|i| i.status.terminal());
        if !terminal {
            return api::error_409("execution has not terminated");
        }
        input["execution_terminal"] = json!(true);
    }
    input["execution_id"] = json!(assignment.index.id);
    match call(&state, input).await {
        Ok(value) => Json(value).into_response(),
        Err(_) => api::response(RpcReply::error(
            503,
            "resource authority unavailable; ownership retained",
        )),
    }
}

/// A resource-waiting dispatch has never reached the node. Settle cancellation
/// under the same lock as submit, only after the authority confirms no grant.
pub(crate) async fn cancel_waiting(
    state: &Arc<AppState>,
    id: &str,
) -> anyhow::Result<Option<RpcReply>> {
    let _lock = state.fleet.request_lock("execution", id).await?;
    let Some(assignment) = state.fleet.assignment(id).await? else {
        return Ok(None);
    };
    if assignment.request.input.get("_resource_request").is_none() {
        return Ok(None);
    }
    let Some(receipt) = state.fleet.receipt("execution", id).await? else {
        return Ok(None);
    };
    if receipt.phase != "prepared" {
        return Ok(None);
    }
    let value = call(state, json!({"action":"inspect","execution_id":id})).await?;
    if value["allocated"] != false {
        return Ok(None);
    }
    let mut index = assignment.index;
    index.status = ExecutionStatus::Cancelled;
    state.fleet.put_index(&index).await?;
    let reply = RpcReply {
        status: 202,
        body: json!(index),
    };
    state
        .fleet
        .finish_dispatch(id, &receipt.fingerprint, &reply)
        .await?;
    Ok(Some(reply))
}
