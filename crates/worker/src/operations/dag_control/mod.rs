use crate::Worker;
use anyhow::Result;
#[cfg(not(windows))]
use opencoder_core::fleet::ExecutionStatus;
use opencoder_core::fleet::{ExecutionKind, RpcReply};
#[cfg(not(windows))]
use serde_json::json;
use serde_json::Value;

#[cfg_attr(windows, allow(unused_variables))]
pub(super) async fn handle(
    worker: &Worker,
    id: &str,
    action: &str,
    input: Value,
) -> Result<RpcReply> {
    let gate = worker.lifecycle_gate(id).await;
    let _guard = gate.lock().await;
    let record = worker.inner.journal.lock().await.get(id).await?;
    let Some(record) = record else {
        return Ok(RpcReply::error(404, "execution not found"));
    };
    if record.assignment.request.kind != ExecutionKind::Dag {
        return Ok(RpcReply::error(
            400,
            "step control requires a DAG execution",
        ));
    }
    let root = crate::layout::dag::accepted_parent(&record)?;
    #[cfg(windows)]
    {
        // The native DAG runtime is Linux-only and the Windows workload
        // stub rejects DAG executions, so step control has nothing to drive.
        Ok(RpcReply::error(
            400,
            "step control requires the native DAG runtime",
        ))
    }
    #[cfg(not(windows))]
    {
        if action == "step-status" {
            if let Some(operation) = input.get("operation_id") {
                let Some(operation) = operation.as_str() else {
                    return Ok(RpcReply::error(400, "operation_id must be a string"));
                };
                return Ok(
                    match opencoder_dag_runtime::control::receipt(&root, id, operation) {
                        Ok(Some(receipt)) => RpcReply::ok(json!(receipt)),
                        Ok(None) => RpcReply::error(404, "step operation not found"),
                        Err(error) => RpcReply::error(400, error.to_string()),
                    },
                );
            }
            return Ok(RpcReply::ok(
                json!({"id":id,"steps":opencoder_dag_runtime::control::status(&root,id)?}),
            ));
        }
        let mut value = input;
        if !value.is_object() {
            return Ok(RpcReply::error(400, "step command input must be an object"));
        }
        value["action"] = json!(action.strip_prefix("step-").unwrap_or(""));
        let command: opencoder_dag::control::Command = match serde_json::from_value(value) {
            Ok(command) => command,
            Err(error) => return Ok(RpcReply::error(400, error.to_string())),
        };
        if let Err(error) = command.validate() {
            return Ok(RpcReply::error(400, error));
        }
        if let Some(prior) =
            opencoder_dag_runtime::control::receipt(&root, id, &command.operation_id)?
        {
            return Ok(if prior.command == command {
                RpcReply::ok(json!(prior))
            } else {
                RpcReply::error(409, "operation ID already has different input")
            });
        }
        if record.assignment.index.status != ExecutionStatus::Running {
            return Ok(RpcReply::error(
                409,
                "DAG must be running; resume the original execution before step control",
            ));
        }
        let Some(definition) = record.assignment.definition.as_ref() else {
            return Ok(RpcReply::error(409, "DAG definition unavailable"));
        };
        let spec = opencoder_dag::decode_spec(definition.get("spec").unwrap_or(definition))
            .map_err(anyhow::Error::msg)?;
        Ok(
            match opencoder_dag_runtime::control::submit(&root, id, &spec, command) {
                Ok(receipt) => RpcReply::ok(json!(receipt)),
                Err(error) => RpcReply::error(409, error.to_string()),
            },
        )
    }
}
