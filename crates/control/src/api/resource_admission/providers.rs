//! Immutable named authorities; each assignment retains its original authority.
use super::Provider;
use crate::{api, AppState};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use opencoder_core::fleet::CreateExecution;
use serde_json::json;
use std::sync::Arc;

const NAMED: &str = "resource_admission_named";
const FROZEN: &str = "resource_admission_assignments";

pub(super) fn valid(value: &Provider) -> bool {
    value.token_file.is_absolute()
        && reqwest::Url::parse(&value.endpoint).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
}

fn key(name: &str, version: u64) -> anyhow::Result<String> {
    anyhow::ensure!(
        opencoder_core::fleet::valid_id(name) && version > 0,
        "invalid provider identity"
    );
    Ok(format!("{name}-v{version}"))
}

pub async fn save(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, u64)>,
    Json(value): Json<Provider>,
) -> Response {
    let key = match key(&name, version) {
        Ok(key) if valid(&value) => key,
        _ => return api::error_400("invalid resource provider configuration".into()),
    };
    let result = async {
        let _lock = state.fleet.request_lock(NAMED, &key).await?;
        let value = json!(value);
        if let Some(previous) = state.fleet.definition(NAMED, &key).await? {
            if previous != value {
                return Ok(false);
            }
        } else {
            state.fleet.put_definition(NAMED, &key, &value).await?;
        }
        anyhow::Ok(true)
    }
    .await;
    match result {
        Ok(true) => Json(json!({"name":name,"version":version,"enabled":true})).into_response(),
        Ok(false) => api::error_409("provider version is immutable"),
        Err(_) => api::error_500("resource provider configuration unavailable".into()),
    }
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, u64)>,
) -> Response {
    let key = match key(&name, version) {
        Ok(value) => value,
        Err(_) => return api::error_400("invalid provider identity".into()),
    };
    match state.fleet.definition(NAMED, &key).await {
        Ok(Some(_)) => Json(json!({"name":name,"version":version,"enabled":true})).into_response(),
        Ok(None) => api::error_404("resource provider not found"),
        Err(_) => api::error_500("resource provider configuration unavailable".into()),
    }
}

/// Called under the execution request lock, before persisting the assignment.
/// A companion definition keeps credential paths on the control plane only.
pub(crate) async fn freeze(state: &AppState, request: &CreateExecution) -> anyhow::Result<()> {
    if request.input.get("_resource_request").is_none() {
        anyhow::ensure!(
            request.input.get("_resource_provider").is_none(),
            "provider requires a resource request"
        );
        return Ok(());
    }
    if state.fleet.definition(FROZEN, &request.id).await?.is_some() {
        return Ok(());
    }
    let selector = request.input.get("_resource_provider");
    let config = if let Some(selector) = selector {
        let name = selector["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("provider name required"))?;
        let version = selector["version"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("provider version required"))?;
        state
            .fleet
            .definition(NAMED, &key(name, version)?)
            .await?
            .ok_or_else(|| anyhow::anyhow!("named resource provider is not registered"))?
    } else {
        // Old device assignments may precede initial authority registration.
        let Some(value) = state.fleet.definition(super::NAMESPACE, "current").await? else {
            return Ok(());
        };
        value
    };
    state
        .fleet
        .put_definition(
            FROZEN,
            &request.id,
            &json!({"provider":selector,"configuration":config}),
        )
        .await?;
    Ok(())
}

pub(super) async fn resolve(state: &AppState, id: &str) -> anyhow::Result<Provider> {
    if let Some(value) = state.fleet.definition(FROZEN, id).await? {
        return Ok(serde_json::from_value(value["configuration"].clone())?);
    }
    // Existing assignments are frozen at their first post-upgrade interaction.
    let _lock = state.fleet.request_lock(FROZEN, id).await?;
    let assignment = state
        .fleet
        .assignment(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("resource assignment not found"))?;
    freeze(state, &assignment.request).await?;
    let value = state
        .fleet
        .definition(FROZEN, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("resource admission provider is not configured"))?;
    Ok(serde_json::from_value(value["configuration"].clone())?)
}
