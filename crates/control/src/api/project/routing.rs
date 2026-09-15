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
