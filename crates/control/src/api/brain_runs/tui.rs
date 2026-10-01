use super::catalog;
use crate::{api::error_500, AppState};
use axum::{extract::State, response::IntoResponse, response::Response, Json};
use opencoder_core::{fleet::ExecutionKind, harness::ServerCapability};
use serde_json::{json, Value};
use std::sync::Arc;

pub fn project(values: Vec<Value>) -> Vec<ServerCapability> {
    let mut cards: Vec<_> = values
        .into_iter()
        .filter(|value| value.get("unavailable_reason").is_none())
        .filter_map(|value| serde_json::from_value::<ServerCapability>(value).ok())
        .filter(|card| matches!(card.kind, ExecutionKind::Agent | ExecutionKind::Operator))
        .filter(|card| !card.id.is_empty() && !card.target.is_empty() && card.id != "self")
        .collect();
    cards.sort_by(|a, b| a.id.cmp(&b.id));
    cards.dedup_by(|a, b| a.id == b.id);
    cards
}

pub async fn list(State(state): State<Arc<AppState>>) -> Response {
    match catalog::capabilities(&state).await {
        Ok(values) => Json(json!({"capabilities":project(values)})).into_response(),
        Err(error) => error_500(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_only_exposes_available_agent_operator_public_fields() {
        let cards = project(vec![
            json!({"id":"ops","kind":"operator","target":"codex","summary":"Ops","definition":{"secret":"private"}}),
            json!({"id":"act","kind":"agent","target":"act","summary":"Act"}),
            json!({"id":"dag","kind":"dag","target":"dag","summary":"Dag"}),
            json!({"id":"bad","kind":"agent","target":"missing","summary":"Bad","unavailable_reason":"missing"}),
        ]);
        assert_eq!(cards.len(), 2);
        let value = serde_json::to_value(&cards).unwrap();
        assert!(value[1].get("definition").is_none());
        assert_eq!(value[1]["target"], "codex");
    }
}
