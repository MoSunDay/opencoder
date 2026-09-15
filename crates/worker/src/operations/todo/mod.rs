mod control;
mod read;

pub(crate) use control::{recover_locked, Control};

use crate::Worker;
use anyhow::Result;
use opencoder_core::fleet::*;
use serde_json::Value;

pub(super) async fn handle(
    worker: &Worker,
    execution: &ExecutionRef,
    action: &str,
    input: Value,
) -> Result<RpcReply> {
    if execution.kind != ExecutionKind::Todos {
        return Ok(RpcReply::error(
            400,
            "TODO operation requires a TODO workflow",
        ));
    }
    match action {
        "todo-review" => read::query(worker, &execution.id, input).await,
        "todo-rerun" => control::accept(worker, &execution.id, input).await,
        _ => Ok(RpcReply::error(400, "unknown TODO operation")),
    }
}
