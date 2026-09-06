use crate::Worker;
use anyhow::Result;
use opencoder_core::fleet::{NodeAdmissionCommand, RpcReply};
use serde_json::json;

pub(super) async fn update(worker: &Worker, command: NodeAdmissionCommand) -> Result<RpcReply> {
    match command {
        NodeAdmissionCommand::Freeze => worker.freeze_admission().await?,
        NodeAdmissionCommand::Reopen => {
            if let Err(error) = worker.reopen_admission().await {
                return Ok(RpcReply::error(503, format!("{error:#}")));
            }
        }
        NodeAdmissionCommand::Status => {}
    }
    Ok(RpcReply::ok(json!({
        "mode": if worker.admission_open() { "open" } else { "frozen" },
        "active_runs": worker.inner.max_runs - worker.inner.slots.available_permits(),
        "owned_processes": opencoder_session::process::active_owned_processes(),
    })))
}
