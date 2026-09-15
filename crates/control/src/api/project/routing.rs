//! Freeze keyed Project routing before a model-dependent choice can be replayed.
use crate::AppState;
use opencoder_core::fleet::RpcReply;
use opencoder_store::fleet::handoff::Receipt;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) async fn brain_preresolve(
    state: &Arc<AppState>,
    todo: &str,
    action: &str,
    input: Value,
) -> Result<Value, RpcReply> {
    if action != "execute" {
        return Ok(input);
    }
    let run_id = input["run_id"].as_str().map(str::to_owned);
    let key = run_id.clone().unwrap_or_else(|| format!("project-{todo}"));
    let _lock = state
        .fleet
        .request_lock("project-routing", &key)
        .await
        .map_err(failure)?;
    // Legacy callers without a run ID submit distinct operations. Their long
    // routing calls still serialize, while keyed retries use a durable choice.
    let Some(_) = run_id else {
        return super::resolve_brain_executor(state, todo, action, input).await;
    };
    let fingerprint =
        opencoder_core::token_hash(&json!({"todo":todo,"action":action,"input":input}).to_string());
    if !state
        .fleet
        .claim_request("project-routing", &key, &fingerprint)
        .await
        .map_err(failure)?
    {
        return Err(RpcReply::error(
            409,
            "project run id already used with different input",
        ));
    }
    if let Some(receipt) = state
        .fleet
        .receipt("project-routing", &key)
        .await
        .map_err(failure)?
    {
        if receipt.phase == "prepared" {
            return Ok(receipt.payload);
        }
    }
    let resolved = super::resolve_brain_executor(state, todo, action, input).await?;
    state
        .fleet
        .save_receipt(
            "project-routing",
            &key,
            &Receipt {
                fingerprint,
                phase: "prepared".into(),
                payload: resolved.clone(),
            },
        )
        .await
        .map_err(failure)?;
    Ok(resolved)
}

fn failure(error: impl std::fmt::Display) -> RpcReply {
    RpcReply::error(500, format!("project routing receipt: {error}"))
}

/// A rejected initial run remains rejected even after a different run creates
/// the Project session. Retry checks happen before any node control operation.
pub(crate) async fn initial_receipt(
    state: &Arc<AppState>,
    todo: &str,
    action: &str,
    input: &Value,
) -> Result<Option<RpcReply>, RpcReply> {
    use opencoder_core::fleet::{CreateExecution, ExecutionKind};
    let Some(run_id) = input["run_id"].as_str() else {
        return Ok(None);
    };
    let Some(receipt) = state
        .fleet
        .receipt("execution", run_id)
        .await
        .map_err(failure)?
    else {
        return Ok(None);
    };
    let input = brain_preresolve(state, todo, action, input.clone()).await?;
    let mut request_input = input.clone();
    request_input.as_object_mut().map(|o| o.remove("node_id"));
    request_input["action"] = json!(action);
    let request = CreateExecution {
        id: format!("project-{todo}"),
        kind: ExecutionKind::Project,
        target: Some(todo.into()),
        node_id: input["node_id"].as_str().map(str::to_owned),
        input: request_input,
    };
    let fingerprint =
        opencoder_core::token_hash(&serde_json::to_string(&request).map_err(failure)?);
    if receipt.fingerprint != fingerprint {
        return Err(RpcReply::error(
            409,
            "project run id already used with different input",
        ));
    }
    if matches!(receipt.phase.as_str(), "accepted" | "rejected") {
        return serde_json::from_value(receipt.payload)
            .map(Some)
            .map_err(failure);
    }
    Ok(None)
}
