use crate::{
    api::{error_400, error_500, response},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::Response,
    Json,
};
use opencoder_core::{
    brain::layered::{LayeredPlan, LayeredRequest, LAYERED_SCHEMA_VERSION},
    brain::*,
    fleet::*,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn save(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let mut body: PlanVersion<Value> = match serde_json::from_value(body) {
        Ok(body) => body,
        Err(error) => return error_400(error.to_string()),
    };
    match validate_layered(&state, body.plan.clone()).await {
        Ok(plan) => body.plan = json!(plan),
        Err(error) => return error_400(error.to_string()),
    }
    match state.fleet.save_brain_plan_document(&body).await {
        Ok(definition) => response(RpcReply::ok(
            json!({"definition":definition,"version":body}),
        )),
        Err(error) => response(RpcReply::error(409, error.to_string())),
    }
}

async fn validate_layered(state: &Arc<AppState>, value: Value) -> anyhow::Result<LayeredPlan> {
    let plan: LayeredPlan = serde_json::from_value(value)?;
    opencoder_brain::layered::validate_plan(&plan)?;
    let request = LayeredRequest {
        schema_version: LAYERED_SCHEMA_VERSION,
        inputs: plan.inputs.clone(),
        plan: plan.clone(),
        artifacts: Default::default(),
        origin: None,
        parent: None,
        depth: 0,
    };
    opencoder_brain::layered::validate_request(&request)?;
    super::v4::catalog::available(state, &request).await?;
    Ok(plan)
}

pub async fn validate(State(state): State<Arc<AppState>>, Json(plan): Json<Value>) -> Response {
    let result = validate_layered(&state, plan).await;
    match result {
        Ok(_) => response(RpcReply::ok(json!({"valid":true}))),
        Err(e) => error_400(e.to_string()),
    }
}

#[derive(Default, Deserialize)]
pub struct PlanQuery {
    pub q: Option<String>,
    pub before: Option<u64>,
    pub from: Option<u64>,
    pub to: Option<u64>,
}
pub async fn list(State(state): State<Arc<AppState>>, Query(query): Query<PlanQuery>) -> Response {
    let result = async {
        let mut definitions = state.fleet.definitions("brain_plan").await?;
        for definition in &mut definitions {
            if let (Some(id), Some(version)) = (
                definition["id"].as_str(),
                definition["latest_version"].as_u64(),
            ) {
                if let Some(plan) = state.fleet.brain_plan_document(id, version).await? {
                    definition["schema_version"] = plan.plan["schema_version"].clone();
                }
            }
        }
        Ok::<_, anyhow::Error>(definitions)
    }
    .await;
    match result {
        Ok(definitions) => response(RpcReply::ok(
            json!({"plans":definitions.into_iter().filter(|p|query.q.as_ref().is_none_or(|q|p.to_string().to_lowercase().contains(&q.to_lowercase()))).collect::<Vec<_>>()}),
        )),
        Err(e) => error_500(e.to_string()),
    }
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    Path((id, version)): Path<(String, u64)>,
) -> Response {
    match state.fleet.brain_plan_document(&id, version).await {
        Ok(Some(p)) => response(RpcReply::ok(json!(p))),
        Ok(None) => response(RpcReply::error(404, "plan version not found")),
        Err(e) => error_500(e.to_string()),
    }
}
pub async fn versions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<PlanQuery>,
) -> Response {
    match state.fleet.brain_plan_documents(&id, query.before).await {
        Ok(versions) => response(RpcReply::ok(json!({"versions":versions}))),
        Err(e) => error_500(e.to_string()),
    }
}
pub async fn stable(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Some(version) = body["version"].as_u64() else {
        return error_400("version is required".into());
    };
    match state.fleet.brain_plan_document(&id, version).await {
        Ok(Some(p)) if p.plan["schema_version"] != LAYERED_SCHEMA_VERSION => {
            return response(RpcReply::error(
                409,
                opencoder_core::brain::layered::LAYERED_MIGRATION,
            ));
        }
        Ok(_) => {}
        Err(e) => return error_500(e.to_string()),
    }
    match state.fleet.mark_brain_stable(&id, version).await {
        Ok(p) => response(RpcReply::ok(json!(p))),
        Err(e) => error_400(e.to_string()),
    }
}
pub async fn diff(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<PlanQuery>,
) -> Response {
    let result = async {
        let a = state
            .fleet
            .brain_plan_document(
                &id,
                query.from.ok_or_else(|| anyhow::anyhow!("from required"))?,
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!("from version not found"))?;
        let b = state
            .fleet
            .brain_plan_document(&id, query.to.ok_or_else(|| anyhow::anyhow!("to required"))?)
            .await?
            .ok_or_else(|| anyhow::anyhow!("to version not found"))?;
        let changes = diff_values(
            "",
            &serde_json::to_value(&a.plan)?,
            &serde_json::to_value(&b.plan)?,
        );
        Ok::<_, anyhow::Error>(
            json!({"from":a.version,"to":b.version,"changelog":b.changelog,"changes":changes}),
        )
    }
    .await;
    match result {
        Ok(v) => response(RpcReply::ok(v)),
        Err(e) => error_400(e.to_string()),
    }
}
fn diff_values(path: &str, a: &Value, b: &Value) -> Vec<Value> {
    if a == b {
        return vec![];
    }
    if let (Some(a), Some(b)) = (a.as_object(), b.as_object()) {
        let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
        keys.into_iter()
            .flat_map(|k| {
                diff_values(
                    &format!("{path}/{}", k.replace('~', "~0").replace('/', "~1")),
                    a.get(k).unwrap_or(&Value::Null),
                    b.get(k).unwrap_or(&Value::Null),
                )
            })
            .collect()
    } else {
        vec![json!({"path":path,"before":a,"after":b})]
    }
}
