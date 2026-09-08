use super::{error_400, error_500, response};
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    response::Response,
    Json,
};
use futures::{stream, StreamExt};
use opencoder_core::fleet::*;
use opencoder_core::message::now_ms;
use opencoder_store::ProjectExecutorKind;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn overview(State(state): State<Arc<AppState>>) -> Response {
    let result=async {
        let goals=state.projects.list_goals().await?;
        let milestones=state.projects.list_milestones(None).await?;
        let todos=state.projects.list_todos(None).await?;
        let items:Vec<Value>=stream::iter(todos).map(|todo| {let state=state.clone();async move {
            let mut value=json!(todo);let id=format!("project-{}",todo.id);
            if let Some(index)=state.fleet.index(&id).await? {
                value["execution"]=json!(index);
                let reply=state.hub.call(&index.node_id,NodeOperation::Inspect{execution:index.execution_ref()}).await;
                if reply.status==200 {
                    for key in ["status","plan_md","active_session_id"] {value[key]=reply.body["todo"][key].clone();}
                } else {value["detail_error"]=reply.body;}
            }
            Ok::<_,anyhow::Error>(value)
        }}).buffered(8).collect::<Vec<_>>().await.into_iter().collect::<anyhow::Result<_>>()?;
        let nested:Vec<_>=goals.into_iter().map(|goal| {
            let mut value=json!(goal);
            value["milestones"]=json!(milestones.iter().filter(|m|m.goal_id==goal.id).map(|milestone| {
                let mut value=json!(milestone);value["todos"]=json!(items.iter().filter(|t|t["milestone_id"]==milestone.id).collect::<Vec<_>>());value
            }).collect::<Vec<_>>());value
        }).collect();
        Ok::<_,anyhow::Error>(json!({"goals":nested,"backlog":items.iter().filter(|t|t["milestone_id"].is_null()).collect::<Vec<_>>()}))
    }.await;
    match result {
        Ok(value) => response(RpcReply::ok(value)),
        Err(error) => error_500(error.to_string()),
    }
}
pub async fn runs(
    State(state): State<Arc<AppState>>,
    Path(todo): Path<String>,
    Query(query): Query<super::executions::ProjectRunsQuery>,
) -> Response {
    if query.before_version.is_some_and(|version| version <= 0) {
        return error_400("invalid project run cursor".into());
    }
    let id = format!("project-{todo}");
    match state.fleet.index(&id).await {
        Ok(None) => response(RpcReply::ok(
            json!({"runs":[],"next_version":null,"more":false}),
        )),
        Ok(Some(_)) => response(
            super::executions::for_id(&state, &id, |execution| NodeOperation::ProjectRuns {
                execution,
                before_version: query.before_version,
            })
            .await,
        ),
        Err(error) => error_500(error.to_string()),
    }
}
pub async fn plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    start(state, id, "plan", body.map(|b| b.0).unwrap_or(json!({}))).await
}
pub async fn execute(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    start(state, id, "execute", body.map(|b| b.0).unwrap_or(json!({}))).await
}
async fn start(state: Arc<AppState>, todo: String, action: &str, input: Value) -> Response {
    if !input.is_object() {
        return error_400("project input must be an object".into());
    }
    if input.get("run_id").is_some_and(|id| {
        !id.as_str()
            .is_some_and(|id| id.starts_with("prun-") && valid_id(id))
    }) {
        return error_400("invalid project run id".into());
    }
    let id = format!("project-{todo}");
    match state.fleet.index(&id).await {
        Ok(Some(index)) => {
            let reply = super::executions::dispatch_command(
                &state,
                &id,
                ExecutionCommand {
                    action: action.into(),
                    input: input.clone(),
                },
            )
            .await;
            if reply.status == 404
                && reply.body["error"] == "execution not found"
                && index.status == ExecutionStatus::Pending
            {
                submit_start(state, todo, action, input).await
            } else {
                response(reply)
            }
        }
        Ok(None) => submit_start(state, todo, action, input).await,
        Err(e) => error_500(e.to_string()),
    }
}
async fn submit_start(state: Arc<AppState>, todo: String, action: &str, input: Value) -> Response {
    let id = format!("project-{todo}");

    // Byte-identical to the pre-brain shape for non-brain todos:
    // `{"action": …}`; a resolved brain todo adds its override key.
    let input = match brain_preresolve(&state, &todo, action, input).await {
        Ok(input) => input,
        Err(reply) => return response(reply),
    };
    let mut request_input = input.clone();
    request_input.as_object_mut().map(|o| o.remove("node_id"));
    request_input["action"] = json!(action);
    if let Some(brain) = input.get("brain").filter(|v| !v.is_null()) {
        request_input["brain"] = brain.clone();
    }
    response(
        super::executions::submit(
            &state,
            CreateExecution {
                id,
                kind: ExecutionKind::Project,
                target: Some(todo),
                node_id: input["node_id"].as_str().map(str::to_owned),
                input: request_input,
            },
        )
        .await,
    )
}

/// Brain pre-resolution for `execute`: a brain todo without a resolvable
/// executor must fail HERE (nodes cannot route), a routed one carries the
/// override in `input.brain`. Unknown todos pass through so
/// `executions::submit` reports the canonical 404; store failures 500
/// HERE instead of being swallowed. `plan` never resolves — planning is
/// executor-agnostic.
pub(super) async fn brain_preresolve(
    state: &Arc<AppState>,
    todo: &str,
    action: &str,
    mut input: Value,
) -> Result<Value, RpcReply> {
    if action != "execute" {
        return Ok(input);
    }
    let record = match state.projects.get_todo(todo).await {
        Ok(Some(record)) => record,
        Ok(None) => return Ok(input),
        Err(e) => return Err(RpcReply::error(500, format!("load todo: {e}"))),
    };
    if record.executor_kind != ProjectExecutorKind::Brain {
        return Ok(input);
    }
    // Pinned capability wins (no LLM round-trip, no plan); otherwise route
    // the todo's title+draft+plan situation through the brain runtime.
    let pinned = record
        .executor_ref
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let (capability_id, plan_id) = match pinned {
        Some(capability_id) => (capability_id.to_string(), None),
        None => {
            // An empty capability library is a normal first-use state —
            // route to the default agent instead of failing dispatch_or_plan
            // (same first-use default as the brain dispatch surface). If the
            // listing itself fails, fall through to dispatch_or_plan which
            // surfaces the store error.
            if let Ok(caps) = state.store.list_brain_capabilities().await {
                if caps.is_empty() {
                    input["brain"] =
                        json!({"kind":"agent","ref":"act","capability_id":null,"plan_id":null});
                    return Ok(input);
                }
            }
            let situation = format!(
                "{}\n\n{}\n\n{}",
                record.title,
                record.draft,
                record.plan_md.as_deref().unwrap_or("")
            );
            let dispatched = state
                .brain
                .dispatch_or_plan(state.brain.chat_model(), &situation, 8, false, now_ms())
                .await
                .map_err(|e| RpcReply::error(400, format!("brain routing failed: {e:#}")))?;
            (dispatched.outcome.capability_id, Some(dispatched.record.id))
        }
    };
    // Capability → executor binding; an unbound capability falls back to
    // the default agent (same default as the brain dispatch surface).
    let target = match state
        .fleet
        .definition("capability_target", &capability_id)
        .await
    {
        Ok(Some(value)) => serde_json::from_value::<CapabilityTarget>(value)
            .map_err(|e| RpcReply::error(500, format!("capability target binding corrupt: {e}")))?,
        Ok(None) => CapabilityTarget {
            kind: ExecutionKind::Agent,
            target: "act".into(),
        },
        Err(e) => return Err(RpcReply::error(503, format!("load capability target: {e}"))),
    };
    input["brain"] = brain_override(&target, &capability_id, plan_id.as_deref())?;
    Ok(input)
}

/// Pure tail of brain pre-resolution: map a routed capability's target onto
/// the override JSON carried in the execution input (the
/// `opencoder_project::service::ExecutorOverride` wire form — kind + ref +
/// provenance). Agent passes its target as the agent name (even the
/// "act" default), team/dag map ref=target; other execution kinds cannot
/// run as a project todo executor.
fn brain_override(
    target: &CapabilityTarget,
    capability_id: &str,
    plan_id: Option<&str>,
) -> Result<Value, RpcReply> {
    let kind = match target.kind {
        ExecutionKind::Agent => ProjectExecutorKind::Agent,
        ExecutionKind::Team => ProjectExecutorKind::Team,
        ExecutionKind::Dag => ProjectExecutorKind::Dag,
        other => {
            return Err(RpcReply::error(
                400,
                format!(
                    "capability target kind {} cannot run as a project todo executor",
                    other.prefix()
                ),
            ))
        }
    };
    Ok(json!({
        "kind": kind.as_str(),
        "ref": target.target,
        "capability_id": capability_id,
        "plan_id": plan_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(kind: ExecutionKind, target: &str) -> CapabilityTarget {
        CapabilityTarget {
            kind,
            target: target.into(),
        }
    }

    #[test]
    fn brain_override_maps_targets_and_carries_provenance() {
        let ov = brain_override(
            &target(ExecutionKind::Agent, "act"),
            "cap-1",
            Some("plan-9"),
        )
        .unwrap();
        assert_eq!(
            ov,
            json!({"kind":"agent","ref":"act","capability_id":"cap-1","plan_id":"plan-9"})
        );
        let ov = brain_override(&target(ExecutionKind::Team, "crew"), "cap-2", None).unwrap();
        assert_eq!(ov["kind"], json!("team"));
        assert_eq!(ov["ref"], json!("crew"));
        assert_eq!(ov["plan_id"], json!(null));
        let ov = brain_override(&target(ExecutionKind::Dag, "etl"), "cap-3", None).unwrap();
        assert_eq!(ov["kind"], json!("dag"));
        assert_eq!(ov["ref"], json!("etl"));
    }

    #[test]
    fn brain_override_rejects_non_executor_kinds() {
        for kind in [
            ExecutionKind::Todos,
            ExecutionKind::Project,
            ExecutionKind::Maintenance,
            ExecutionKind::System,
        ] {
            let reply = brain_override(&target(kind, "x"), "cap", None).unwrap_err();
            assert_eq!(reply.status, 400, "{kind:?}");
            assert!(
                reply.body["error"]
                    .as_str()
                    .unwrap()
                    .contains("cannot run as a project todo executor"),
                "{kind:?}: {}",
                reply.body
            );
        }
    }
}
