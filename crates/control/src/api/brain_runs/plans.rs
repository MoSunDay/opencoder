use crate::{
    api::{error_400, error_500, response},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::Response,
    Json,
};
use opencoder_core::{brain::*, fleet::*};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn pin(state: &Arc<AppState>, plan: &mut OntologyPlan) -> anyhow::Result<()> {
    opencoder_brain::ontology::validate(plan)?;
    let config = opencoder_core::Config::load(&state.workdir)?;
    for step in &mut plan.steps {
        if step.action.definition.is_none() {
            let request = CreateExecution {
                id: format!("{}-preflight", step.action.kind.prefix()),
                kind: step.action.kind,
                target: Some(step.action.target.clone()),
                input: json!({}),
                node_id: step.action.node_id.clone(),
            };
            step.action.definition = crate::api::catalog::resolve(state, &request)
                .await
                .map_err(|r| anyhow::anyhow!("{}: {}", step.id, r.body))?;
        }
        if step.action.runtime.is_none() {
            step.action.runtime = Some(serde_json::to_value(&config.agent.runtime)?);
        }
        if step.action.codex.is_none() {
            step.action.codex = config
                .agent
                .codex
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?;
        }
        let names = agent_names(&step.action)?;
        for name in names {
            if !step.action.agent_manifests.contains_key(&name) {
                step.action.agent_manifests.insert(
                    name.clone(),
                    opencoder_core::agent::scope::with_root_sync(
                        config.agent.agents_dir.clone(),
                        || opencoder_core::brain::resources::agent_manifest(&name),
                    )
                    .map_err(anyhow::Error::msg)?,
                );
            }
        }
    }
    Ok(())
}
fn agent_names(action: &ActionSpec) -> anyhow::Result<Vec<String>> {
    let definition = action.definition.as_ref().unwrap_or(&Value::Null);
    Ok(match action.kind {
        ExecutionKind::Agent => vec![action.target.clone()],
        ExecutionKind::Team => serde_json::from_value::<TeamDefinition>(definition.clone())?
            .members
            .into_iter()
            .map(|m| m.agent)
            .collect(),
        ExecutionKind::Todos => {
            let spec: opencoder_todos::WorkflowSpec = serde_json::from_value(definition.clone())?;
            let mut names = vec!["workflow".into()];
            names.extend(spec.todos.into_iter().map(|s| s.agent));
            names
        }
        ExecutionKind::Dag => {
            let spec = opencoder_dag::decode_spec(definition.get("spec").unwrap_or(definition))
                .map_err(anyhow::Error::msg)?;
            spec.steps
                .into_iter()
                .filter_map(|s| match s.kind {
                    opencoder_dag::StepKind::Agent { agent, .. } => {
                        Some(agent.unwrap_or_else(|| "act".into()))
                    }
                    opencoder_dag::StepKind::Runner { agent, .. } => Some(agent),
                    _ => None,
                })
                .collect()
        }
        _ => anyhow::bail!("unsupported business kind"),
    })
}

pub async fn save(
    State(state): State<Arc<AppState>>,
    Json(mut body): Json<PlanVersion>,
) -> Response {
    if let Err(error) = pin(&state, &mut body.plan).await {
        return error_400(format!("{error:#}"));
    }
    match state.fleet.save_brain_plan(&body).await {
        Ok(definition) => response(RpcReply::ok(
            json!({"definition":definition,"version":body}),
        )),
        Err(error) => response(RpcReply::error(409, error.to_string())),
    }
}

pub async fn validate(Json(plan): Json<OntologyPlan>) -> Response {
    match opencoder_brain::ontology::validate(&plan) {
        Ok(()) => response(RpcReply::ok(json!({"valid":true}))),
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
    match state.fleet.definitions("brain_plan").await {
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
    match state.fleet.brain_plan_version(&id, version).await {
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
    match state.fleet.brain_plan_versions(&id, query.before).await {
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
            .brain_plan_version(
                &id,
                query.from.ok_or_else(|| anyhow::anyhow!("from required"))?,
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!("from version not found"))?;
        let b = state
            .fleet
            .brain_plan_version(&id, query.to.ok_or_else(|| anyhow::anyhow!("to required"))?)
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
