use crate::{
    api::{executions, response},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::Response,
    Json,
};
use opencoder_core::fleet::ExecutionCommand;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Default, Deserialize, Serialize)]
pub struct ReviewQuery {
    pub section: Option<String>,
    pub todo_id: Option<String>,
    pub session_id: Option<String>,
    pub after_seq: Option<i64>,
    pub before_seq: Option<i64>,
    pub message_offset: Option<u64>,
    pub after_ordinal: Option<u64>,
    pub generation: Option<i64>,
    pub offset: Option<u64>,
    pub etag: Option<String>,
}

pub async fn review(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ReviewQuery>,
) -> Response {
    response(
        executions::command_id(
            &state,
            &id,
            ExecutionCommand {
                action: "todo-review".into(),
                input: serde_json::to_value(query).expect("review query serializes"),
            },
        )
        .await,
    )
}

pub async fn rerun(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<Value>,
) -> Response {
    response(
        executions::command_id(
            &state,
            &id,
            ExecutionCommand {
                action: "todo-rerun".into(),
                input,
            },
        )
        .await,
    )
}

pub async fn context_preview(Json(input): Json<Value>) -> Response {
    let spec: opencoder_todos::WorkflowSpec = match serde_json::from_value(input["spec"].clone()) {
        Ok(spec) => spec,
        Err(error) => {
            return response(opencoder_core::fleet::RpcReply::error(
                400,
                error.to_string(),
            ))
        }
    };
    let Some(todo) = spec
        .todos
        .iter()
        .find(|t| Some(t.id.as_str()) == input["todo_id"].as_str())
    else {
        return response(opencoder_core::fleet::RpcReply::error(
            404,
            "TODO not found",
        ));
    };
    let state = opencoder_todos::domain::initial_state(&spec, "preview".into(), "preview".into());
    match opencoder_todos::review::context::dispatch_context(
        &spec,
        &state,
        todo,
        opencoder_todos::types::ContextMode::New,
    ) {
        Ok(context) => response(opencoder_core::fleet::RpcReply::ok(context)),
        Err(error) => response(opencoder_core::fleet::RpcReply::error(
            400,
            error.to_string(),
        )),
    }
}
