//! A convenience adapter to existing capability submission. No project
//! execution loop: the receipt freezes admission and the TODO keeps a link.
use crate::{
    api::{brain_runs, executions, response},
    AppState,
};
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_core::fleet::{CreateExecution, ExecutionKind, RpcReply};
use opencoder_store::fleet::handoff::Receipt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Dispatch {
    pub execution_id: String,
    pub capability_id: String,
    #[serde(default = "empty_input")]
    pub input: Value,
    pub node_id: Option<String>,
}
fn empty_input() -> Value {
    json!({})
}

pub async fn dispatch(
    State(state): State<Arc<AppState>>,
    Path(todo): Path<String>,
    Json(body): Json<Dispatch>,
) -> Response {
    response(match submit(&state, &todo, &body).await {
        Ok(reply) => reply,
        Err(error) => RpcReply::error(500, error.to_string()),
    })
}

async fn submit(state: &Arc<AppState>, todo: &str, body: &Dispatch) -> anyhow::Result<RpcReply> {
    if !opencoder_core::fleet::valid_id(&body.execution_id) || !body.input.is_object() {
        return Ok(RpcReply::error(
            400,
            "valid execution_id and object input required",
        ));
    }
    if state.projects.get_todo(todo).await?.is_none() {
        return Ok(RpcReply::error(404, "todo not found"));
    }
    let scope = "project-dispatch";
    let _lock = state.fleet.request_lock(scope, &body.execution_id).await?;
    let fingerprint = opencoder_core::token_hash(&serde_json::to_string(&(todo, body))?);
    if !state
        .fleet
        .claim_request(scope, &body.execution_id, &fingerprint)
        .await?
    {
        return Ok(RpcReply::error(
            409,
            "execution id already belongs to a different project dispatch",
        ));
    }
    let mut receipt = state
        .fleet
        .receipt(scope, &body.execution_id)
        .await?
        .unwrap();
    if receipt.phase == "claimed" {
        if state.fleet.index(&body.execution_id).await?.is_some()
            || state.fleet.assignment(&body.execution_id).await?.is_some()
        {
            return Ok(RpcReply::error(
                409,
                "execution already exists; link its id instead",
            ));
        }
        let catalog = brain_runs::catalog::capabilities(state).await?;
        let Some(capability) = catalog
            .into_iter()
            .find(|cap| cap["id"] == body.capability_id)
        else {
            return Ok(RpcReply::error(404, "capability not found"));
        };
        let request = match request(body, &capability) {
            Ok(value) => value,
            Err(error) => return Ok(RpcReply::error(400, error.to_string())),
        };
        receipt = Receipt {
            fingerprint,
            phase: "prepared".into(),
            payload: json!({"capability":capability,"request":request}),
        };
        state
            .fleet
            .save_receipt(scope, &body.execution_id, &receipt)
            .await?;
    }
    let kind: ExecutionKind =
        serde_json::from_value(receipt.payload["capability"]["kind"].clone())?;
    let reply = if kind == ExecutionKind::Brain {
        brain_runs::v4::submit(state.clone(), receipt.payload["request"].clone()).await
    } else {
        executions::submit(
            state,
            serde_json::from_value(receipt.payload["request"].clone())?,
        )
        .await
    };
    if reply.status >= 300 {
        return Ok(reply);
    }
    let linked = super::attach(
        state,
        todo,
        &super::LinkBody {
            execution_id: body.execution_id.clone(),
            capability_id: Some(body.capability_id.clone()),
        },
    )
    .await;
    if linked.status >= 300 {
        return Ok(RpcReply {
            status: linked.status,
            body: json!({
                "execution_id":body.execution_id,"accepted":true,"linked":false,"error":linked.body["error"]
            }),
        });
    }
    Ok(RpcReply {
        status: 202,
        body: json!({"execution_id":body.execution_id,"capability_id":body.capability_id,"linked":true}),
    })
}

fn request(body: &Dispatch, capability: &Value) -> anyhow::Result<Value> {
    anyhow::ensure!(
        capability.get("unavailable_reason").is_none() && capability["definition"].is_object(),
        "capability unavailable"
    );
    let kind: ExecutionKind = serde_json::from_value(capability["kind"].clone())?;
    anyhow::ensure!(super::linkable(kind), "unsupported capability kind");
    anyhow::ensure!(
        body.execution_id
            .starts_with(&format!("{}-", kind.prefix())),
        "execution id prefix does not match capability kind"
    );
    let required: Vec<String> = serde_json::from_value(
        capability
            .get("required_inputs")
            .cloned()
            .unwrap_or(json!([])),
    )?;
    opencoder_brain::contracts::validate_inputs(&required, body.input.as_object().unwrap())?;
    if kind == ExecutionKind::Brain {
        anyhow::ensure!(
            capability["definition"]["plan_id"].is_string()
                && capability["definition"]["version"].is_u64(),
            "saved Brain plan required"
        );
        return Ok(
            json!({"id":body.execution_id,"schema_version":7,"node_id":body.node_id,
            "plan":{"id":capability["definition"]["plan_id"],"version":capability["definition"]["version"]},"inputs":body.input}),
        );
    }
    let mut input = body.input.clone();
    if matches!(kind, ExecutionKind::Dag | ExecutionKind::Team) {
        input["definition"] = capability["definition"].clone();
    }
    if kind == ExecutionKind::Todos {
        input["spec"] = capability["definition"].clone();
    }
    Ok(serde_json::to_value(CreateExecution {
        id: body.execution_id.clone(),
        kind,
        target: capability["target"].as_str().map(str::to_owned),
        input,
        node_id: body.node_id.clone(),
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capability_adapter_preserves_each_native_submission_and_rejects_wrong_ids() {
        for kind in ["agent", "operator", "dag", "team", "todos", "brain"] {
            let cap = json!({"id":"cap","kind":kind,"target":"registered","definition":{"plan_id":"saved","version":2},"required_inputs":["prompt"]});
            let mut body = Dispatch {
                execution_id: format!("{kind}-test"),
                capability_id: "cap".into(),
                input: json!({"prompt":"task"}),
                node_id: Some("node".into()),
            };
            let actual = request(&body, &cap).unwrap();
            if kind == "brain" {
                assert_eq!(actual["plan"], json!({"id":"saved","version":2}));
                assert_eq!(actual["inputs"], body.input);
            } else {
                assert_eq!(actual["target"], "registered");
                assert_eq!(actual["input"]["prompt"], "task");
            }
            body.input = json!({});
            assert!(request(&body, &cap).is_err());
            body.execution_id = "unrelated-test".into();
            assert!(request(&body, &cap).is_err());
        }
    }
}
