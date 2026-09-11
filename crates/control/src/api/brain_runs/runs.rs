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

#[derive(Deserialize, serde::Serialize)]
pub struct CreateRun {
    pub id: Option<String>,
    pub node_id: Option<String>,
    pub mode: PlanningMode,
    pub objective: String,
    #[serde(default)]
    pub inputs: std::collections::BTreeMap<String, Value>,
    pub plan: Option<PlanRef>,
    #[serde(default)]
    pub references: Vec<PlanRef>,
}
pub async fn create(State(state): State<Arc<AppState>>, Json(body): Json<CreateRun>) -> Response {
    let intent = serde_json::to_value(&body).expect("serializable request intent");
    if let Some(id) = body.id.as_deref() {
        if let Ok(Some(index)) = state.fleet.index(id).await {
            if index.kind != ExecutionKind::Brain {
                return response(RpcReply::error(409, "execution kind conflict"));
            }
            let receipt = state
                .hub
                .call(
                    &index.node_id,
                    NodeOperation::Brain {
                        execution: index.execution_ref(),
                        action: "intent".into(),
                        input: Value::Null,
                    },
                )
                .await;
            if receipt.status >= 300 && receipt.status != 404 {
                return response(receipt);
            }
            if receipt.status != 404 {
                return response(if receipt.body == intent {
                    RpcReply {
                        status: 202,
                        body: json!(index),
                    }
                } else {
                    RpcReply::error(409, "run id was already accepted with a different intent")
                });
            }
            // The control index may have committed before node admission.
            // No accepted intent exists yet: replay Create with the same ID.
        }
    }
    let request = async {
        anyhow::ensure!(
            body.mode != PlanningMode::Dynamic || body.plan.is_none(),
            "dynamic mode uses references, not a fixed plan"
        );
        let mut references = vec![];
        for reference in body.references {
            references.push(
                state
                    .fleet
                    .brain_plan_version(&reference.id, reference.version)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("reference version not found"))?,
            );
        }
        let plan = match body.plan {
            Some(reference) => Some(
                state
                    .fleet
                    .brain_plan_version(&reference.id, reference.version)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("fixed version not found"))?,
            ),
            None => None,
        };
        let request = BrainRequest {
            mode: body.mode,
            objective: body.objective,
            inputs: body.inputs,
            plan,
            references,
            capabilities: super::catalog::capabilities(&state).await?,
        };
        let id = body
            .id
            .unwrap_or_else(|| format!("brain-{}", ulid::Ulid::new()));
        opencoder_brain::execution::initialize(&id, request.clone(), 0)?;
        Ok::<_, anyhow::Error>(CreateExecution {
            id,
            kind: ExecutionKind::Brain,
            target: None,
            input: {
                let mut value = serde_json::to_value(request)?;
                value["brain_intent"] = intent;
                value
            },
            node_id: body.node_id,
        })
    }
    .await;
    match request {
        Ok(request) => response(crate::api::executions::submit(&state, request).await),
        Err(e) => error_400(e.to_string()),
    }
}
pub async fn list(State(state): State<Arc<AppState>>, Query(page): Query<Page>) -> Response {
    let cursor = match (page.cursor_created_at, page.cursor_id) {
        (None, None) => None,
        (Some(created_at), Some(id)) if valid_id(&id) => Some(ExecutionCursor { created_at, id }),
        _ => {
            return error_400("cursor_created_at and valid cursor_id are required together".into())
        }
    };
    match state
        .fleet
        .indexes_page(None, Some(ExecutionKind::Brain), cursor.as_ref(), 100)
        .await
    {
        Ok(page) => response(RpcReply::ok(
            json!({"runs":page.executions,"next_cursor":page.next_cursor}),
        )),
        Err(e) => error_500(e.to_string()),
    }
}
pub async fn call(state: &Arc<AppState>, id: &str, action: &str, input: Value) -> RpcReply {
    let index = match state.fleet.index(id).await {
        Ok(Some(i)) if i.kind == ExecutionKind::Brain => i,
        Ok(_) => return RpcReply::error(404, "brain run not found"),
        Err(e) => return RpcReply::error(500, e.to_string()),
    };
    state
        .hub
        .call(
            &index.node_id,
            NodeOperation::Brain {
                execution: index.execution_ref(),
                action: action.into(),
                input,
            },
        )
        .await
}
#[derive(Default, Deserialize)]
pub struct Page {
    pub cursor_created_at: Option<i64>,
    pub cursor_id: Option<String>,
    pub step: Option<String>,
    pub offset: Option<u64>,
    pub after: Option<i64>,
}
pub async fn snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    response(
        call(
            &state,
            &id,
            "snapshot",
            json!({"offset":page.offset.unwrap_or(0)}),
        )
        .await,
    )
}
pub async fn context(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    response(call(&state, &id, "context", json!({})).await)
}
pub async fn actions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    response(
        call(
            &state,
            &id,
            "actions",
            json!({"offset":page.offset.unwrap_or(0)}),
        )
        .await,
    )
}
pub async fn events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    response(
        call(
            &state,
            &id,
            "events",
            json!({"after":page.after.unwrap_or(0)}),
        )
        .await,
    )
}
pub async fn instance(
    State(state): State<Arc<AppState>>,
    Path((id, instance)): Path<(String, String)>,
) -> Response {
    response(call(&state, &id, "instance", json!({"id":instance})).await)
}
pub async fn input(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    response(call(&state, &id, "input", body).await)
}
pub async fn command(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(command): Json<ExecutionCommand>,
) -> Response {
    if !matches!(command.action.as_str(), "pause" | "resume" | "cancel") {
        return error_400("supported commands: pause, resume, cancel".into());
    }
    let _gate = state.brain_gate.lock(&id).await;
    response(call(&state, &id, &command.action, command.input).await)
}

pub async fn instances(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    response(
        call(
            &state,
            &id,
            "instances",
            json!({"step":page.step,"offset":page.offset.unwrap_or(0)}),
        )
        .await,
    )
}
