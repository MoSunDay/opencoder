//! On-demand node reads. The control plane never persists these results.
use crate::{
    api::{executions, response},
    AppState,
};
use axum::{
    extract::{Path, State},
    response::Response,
};
use opencoder_core::fleet::{ExecutionKind, NodeOperation, RpcReply};
use serde_json::{json, Value};
use std::sync::Arc;

fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.trim().is_empty() && value.trim() != "null" => {
            Some(value.trim().to_owned())
        }
        Value::Object(value) if !value.is_empty() && !value.contains_key("omitted") => {
            Some(serde_json::to_string_pretty(value).ok()?)
        }
        _ => None,
    }
}

fn bounded(value: String) -> String {
    value.chars().take(64 * 1024).collect()
}

fn direct_conclusion(kind: ExecutionKind, detail: &Value) -> Option<String> {
    let result = &detail["result"];
    match kind {
        ExecutionKind::Agent | ExecutionKind::Operator => text(&result["output_text"]),
        ExecutionKind::Team => {
            text(&detail["topic"]["final_summary"]).or_else(|| text(&result["final_summary"]))
        }
        ExecutionKind::Todos => result["todos"].as_object().and_then(|todos| {
            let rows: Vec<String> = todos
                .iter()
                .filter_map(|(name, todo)| {
                    text(&todo["candidate"]["result"])
                        .or_else(|| text(&todo["candidate"]["summary"]))
                        .map(|value| format!("### {name}\n{value}"))
                })
                .collect();
            (!rows.is_empty()).then(|| rows.join("\n\n"))
        }),
        _ => None,
    }
}

pub async fn get(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    response(read(&state, &id).await)
}

async fn read(state: &AppState, id: &str) -> RpcReply {
    let index = match state.fleet.index(id).await {
        Ok(Some(index)) => index,
        Ok(None) => return RpcReply::error(404, "execution not found"),
        Err(error) => return RpcReply::error(500, error.to_string()),
    };
    let reply = executions::inspect_id(state, id).await;
    if reply.status >= 300 {
        return reply;
    }
    let detail = reply.body;
    let summary = if index.kind == ExecutionKind::Brain {
        let reply = executions::for_id(state, id, |execution| NodeOperation::Brain {
            execution,
            action: "layered_summary".into(),
            input: Value::Null,
        })
        .await;
        if reply.status >= 300 {
            return reply;
        }
        text(&reply.body["summary"])
    } else if index.kind == ExecutionKind::Dag {
        // DAG outputs stay in step detail/artifact APIs; list the steps here
        // instead of silently returning only a subset of a potentially large run.
        text(&detail["result"]["summary"])
    } else {
        direct_conclusion(index.kind, &detail)
    };
    let truncated = summary
        .as_ref()
        .is_some_and(|text| text.chars().count() > 64 * 1024);
    let omitted = detail["result"].get("omitted").is_some();
    RpcReply::ok(json!({
        "execution_id":id,"kind":index.kind,"node_id":index.node_id,
        "status":detail["execution"]["status"],"summary":summary.map(bounded),
        "truncated":truncated,"omitted":omitted,"error":detail["error"],"steps":detail["dag_steps"]["steps"],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_conclusions_use_their_authoritative_result_fields() {
        assert_eq!(
            direct_conclusion(
                ExecutionKind::Agent,
                &json!({"result":{"output_text":" agent answer "}})
            ),
            Some("agent answer".into())
        );
        assert_eq!(
            direct_conclusion(
                ExecutionKind::Operator,
                &json!({"result":{"output_text":"operator answer"}})
            ),
            Some("operator answer".into())
        );
        assert_eq!(
            direct_conclusion(
                ExecutionKind::Team,
                &json!({"topic":{"final_summary":"team answer"},"result":{"final_summary":"older"}})
            ),
            Some("team answer".into())
        );
        assert_eq!(
            direct_conclusion(
                ExecutionKind::Todos,
                &json!({"result":{"todos":{"work":{"candidate":{"result":"done","summary":"summary"}}}}})
            ),
            Some("### work\ndone".into())
        );
    }

    #[test]
    fn missing_native_conclusions_do_not_claim_success() {
        for kind in [
            ExecutionKind::Agent,
            ExecutionKind::Team,
            ExecutionKind::Todos,
        ] {
            assert_eq!(direct_conclusion(kind, &json!({"result":{}})), None);
        }
    }
}
