use crate::{
    api::{executions, response},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use opencoder_core::fleet::NodeOperation;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct Page {
    #[serde(default)]
    offset: usize,
    #[serde(default = "page_size")]
    limit: usize,
}
fn page_size() -> usize {
    100
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Path((id, step)): Path<(String, String)>,
    Query(page): Query<Page>,
) -> Response {
    response(
        executions::for_id(&state, &id, |execution| NodeOperation::DagInstances {
            execution,
            step,
            index: None,
            offset: page.offset,
            limit: page.limit.clamp(1, 200),
        })
        .await,
    )
}

pub async fn detail(
    State(state): State<Arc<AppState>>,
    Path((id, step, index)): Path<(String, String, usize)>,
) -> Response {
    response(
        executions::for_id(&state, &id, |execution| NodeOperation::DagInstances {
            execution,
            step,
            index: Some(index),
            offset: 0,
            limit: 100,
        })
        .await,
    )
}
