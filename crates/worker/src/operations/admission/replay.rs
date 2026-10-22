use crate::Worker;
use opencoder_core::fleet::*;
use serde_json::json;

pub(in crate::operations) async fn accepted_reply(
    worker: &Worker,
    assignment: &mut Assignment,
) -> anyhow::Result<Option<RpcReply>> {
    let Some(existing) = worker
        .inner
        .journal
        .lock()
        .await
        .get(&assignment.index.id)
        .await?
    else {
        return Ok(None);
    };
    if assignment.request.kind == ExecutionKind::Project
        && assignment.request.input.get("run_id").is_none()
    {
        assignment.request.input["run_id"] = existing.assignment.request.input["run_id"].clone();
    }
    if existing.assignment.request != assignment.request
        || existing.assignment.private_context != assignment.private_context
    {
        return Ok(Some(RpcReply::error(
            409,
            "execution id already accepted with different input",
        )));
    }
    let mut body = json!(existing.assignment.index);
    if assignment.request.kind == ExecutionKind::Project {
        body["run_id"] = existing.assignment.request.input["run_id"].clone();
    }
    Ok(Some(RpcReply::ok(body)))
}
