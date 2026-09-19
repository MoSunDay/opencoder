use super::runtime;
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
#[derive(Deserialize)]
pub struct Page {
    pub after: Option<u64>,
    pub limit: Option<u32>,
}
pub type Command = ExecutionCommand;

pub async fn create(State(state): State<Arc<AppState>>, Json(value): Json<Value>) -> Response {
    let mut request_value = value.clone();
    if let Some(object) = request_value.as_object_mut() {
        object.remove("id");
        object.remove("node_id");
    }
    let request: BrainSchedulerRequest = match serde_json::from_value(request_value) {
        Ok(v) => v,
        Err(e) => return error_400(e.to_string()),
    };
    if let Err(e) = opencoder_brain::scheduler::validate_request(&request) {
        return error_400(e.to_string());
    }
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("brain-{}", ulid::Ulid::new()));
    if !valid_id(&id) || !id.starts_with("brain-") {
        return error_400("invalid brain run id".into());
    }
    // Keep the request idempotent across retries and concurrent control-plane
    // callers. The fleet receipt owns the original intent; the scheduler
    // projection is only created by that owner.
    let _lock = match state.fleet.request_lock("brain-run", &id).await {
        Ok(lock) => lock,
        Err(error) => return error_500(error.to_string()),
    };
    let fingerprint = opencoder_core::token_hash(&value.to_string());
    match state
        .fleet
        .claim_request("brain-run", &id, &fingerprint)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return response(RpcReply::error(
                409,
                "run id was already accepted with a different intent",
            ))
        }
        Err(error) => return error_500(error.to_string()),
    }
    let existing = match state.store.brain_scheduler(&id).await {
        Ok(snapshot) => snapshot,
        Err(error) => return error_500(error.to_string()),
    };
    if let Some(existing) = &existing {
        let execution = match state.fleet.index(&id).await {
            Ok(execution) => execution,
            Err(error) => return error_500(error.to_string()),
        };
        if let Some(execution) = execution {
            return response(RpcReply {
                status: 202,
                body: json!({"schema_version":3,"run_id":id,"execution":execution,"phase":existing.run.phase}),
            });
        }
    }
    let change = match opencoder_brain::scheduler::initialize(
        &id,
        &request,
        opencoder_core::message::now_ms(),
    ) {
        Ok(v) => v,
        Err(e) => return error_400(e.to_string()),
    };
    if existing.is_none() {
        if let Err(e) = state.store.commit_brain_scheduler(&change).await {
            return error_500(e.to_string());
        }
    }
    let input = json!({"schema_version":3,"scheduler_request":request,"scheduler_intent":value});
    let reply = crate::api::executions::submit(
        &state,
        CreateExecution {
            id: id.clone(),
            kind: ExecutionKind::Brain,
            target: None,
            input,
            node_id: value
                .get("node_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
    )
    .await;
    if reply.status >= 300 {
        return response(reply);
    }
    tokio::spawn({
        let state = state.clone();
        let id = id.clone();
        async move {
            if let Err(e) = runtime::wake(&state, &id).await {
                tracing::warn!(%e, "v3 scheduler wake failed");
            }
        }
    });
    response(RpcReply {
        status: 202,
        body: json!({"schema_version":3,"run_id":id,"execution":reply.body}),
    })
}
pub async fn snapshot(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.store.brain_scheduler(&id).await {
        Ok(Some(s)) => response(RpcReply::ok(json!(BrainSchedulerSnapshot {
            schema_version: 3,
            run: s.run,
            operations: s.operations
        }))),
        Ok(None) => response(RpcReply::error(404, "brain v3 run not found")),
        Err(e) => error_500(e.to_string()),
    }
}
pub async fn events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Response {
    match state.store.brain_scheduler(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return response(RpcReply::error(404, "brain v3 run not found")),
        Err(e) => return error_500(e.to_string()),
    }
    let limit = page.limit.unwrap_or(100).clamp(1, 500);
    match state
        .store
        .brain_scheduler_events(&id, page.after.unwrap_or(0), limit)
        .await
    {
        Ok(v) => response(RpcReply::ok(
            json!({"events":v,"more":v.len()==limit as usize,"next_seq":v.last().map(|e|e.seq)}),
        )),
        Err(e) => error_500(e.to_string()),
    }
}
pub async fn round(
    State(state): State<Arc<AppState>>,
    Path((id, round)): Path<(String, u32)>,
) -> Response {
    match state.store.brain_scheduler(&id).await {
        Ok(Some(s)) => response(RpcReply::ok(
            json!({"run_id":id,"round":round,"operations":s.operations.into_iter().filter(|o|o.round==round).collect::<Vec<_>>()}),
        )),
        Ok(None) => response(RpcReply::error(404, "brain v3 run not found")),
        Err(e) => error_500(e.to_string()),
    }
}
pub async fn command(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Command>,
) -> Response {
    match state.store.brain_scheduler(&id).await {
        Ok(Some(snapshot)) => {
            let change = match opencoder_brain::scheduler::command(
                &snapshot,
                &body.action,
                opencoder_core::message::now_ms(),
            ) {
                Ok(change) => change,
                Err(error) => return error_400(error.to_string()),
            };
            match state.store.commit_brain_scheduler(&change).await {
                Ok(next) => {
                    if body.action == "cancel" {
                        let _ = runtime::request_cancellations(&state, &next).await;
                    }
                    if body.action == "resume" {
                        let state = state.clone();
                        let id = id.clone();
                        tokio::spawn(async move {
                            let _ = runtime::wake(&state, &id).await;
                        });
                    }
                    response(RpcReply::ok(json!(next)))
                }
                Err(error) => error_500(error.to_string()),
            }
        }
        Ok(None) => response(RpcReply::error(404, "brain v3 run not found")),
        Err(error) => error_500(error.to_string()),
    }
}
