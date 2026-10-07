use crate::AppState;
use opencoder_core::fleet::{Assignment, ExecutionIndex, ExecutionKind, RpcReply};
use serde_json::Value;
use std::sync::Arc;

fn matches(capability: &Value, assignment: &Assignment) -> bool {
    let request = &assignment.request;
    if capability["kind"] != serde_json::json!(request.kind) {
        return false;
    }
    if request.kind == ExecutionKind::Brain {
        return capability["definition"]["plan_id"]
            == request.input["layered_intent"]["plan"]["id"]
            && capability["definition"]["version"]
                == request.input["layered_intent"]["plan"]["version"]
            && capability["definition"]["plan_id"].is_string();
    }
    capability["target"].as_str() == request.target.as_deref() && request.target.is_some()
}

pub(super) async fn resolve(
    state: &Arc<AppState>,
    index: &ExecutionIndex,
    selected: Option<&str>,
) -> Result<Option<String>, RpcReply> {
    let fail = |error: anyhow::Error| RpcReply::error(500, error.to_string());
    let Some(assignment) = state.fleet.assignment(&index.id).await.map_err(fail)? else {
        return if selected.is_some() {
            Err(RpcReply::error(
                409,
                "execution admission record is unavailable",
            ))
        } else {
            Ok(None)
        };
    };
    // A dispatch receipt survives registry edits and never takes its capability
    // identity from arbitrary execution input supplied by an API caller.
    if let Some(receipt) = state
        .fleet
        .receipt("project-dispatch", &index.id)
        .await
        .map_err(fail)?
    {
        let cap = &receipt.payload["capability"];
        if matches(cap, &assignment) {
            let id = cap["id"].as_str();
            if selected.is_some_and(|selected| Some(selected) != id) {
                return Err(RpcReply::error(
                    409,
                    "capability differs from the admitted execution",
                ));
            }
            return Ok(id.map(str::to_owned));
        }
    }
    let catalog = crate::api::brain_runs::catalog::capabilities(state)
        .await
        .map_err(fail)?;
    let cap = catalog
        .iter()
        .find(|cap| matches(cap, &assignment) && selected.is_none_or(|id| cap["id"] == id));
    match (cap, selected) {
        (Some(cap), _) => Ok(cap["id"].as_str().map(str::to_owned)),
        (None, Some(_)) => Err(RpcReply::error(
            409,
            "capability does not match the admitted execution",
        )),
        (None, None) => Ok(None),
    }
}
