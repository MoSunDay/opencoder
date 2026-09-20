use crate::Worker;
use opencoder_core::fleet::*;
use serde_json::json;

pub(in crate::operations) async fn accepted_reply(
    worker: &Worker,
    assignment: &mut Assignment,
) -> Option<RpcReply> {
    let existing = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(&assignment.index.id)
        .cloned()?;
    if assignment.request.kind == ExecutionKind::Project
        && assignment.request.input.get("run_id").is_none()
    {
        assignment.request.input["run_id"] = existing.assignment.request.input["run_id"].clone();
    }
    if existing.assignment.request != assignment.request {
        return Some(RpcReply::error(
            409,
            "execution id already accepted with different input",
        ));
    }
    let mut body = json!(existing.assignment.index);
    if assignment.request.kind == ExecutionKind::Project {
        body["run_id"] = existing.assignment.request.input["run_id"].clone();
    }
    Some(RpcReply::ok(body))
}
