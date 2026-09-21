use crate::Worker;
use anyhow::{ensure, Result};
use opencoder_core::fleet::*;
use serde_json::{json, Value};

pub async fn handle(
    worker: &Worker,
    reference: &ExecutionRef,
    action: &str,
    input: Value,
) -> Result<RpcReply> {
    ensure!(valid_id(&reference.id), "invalid execution id");
    if action == "capability_probe" {
        let config = worker.configuration()?;
        let matches = opencoder_core::agent::scope::with_root_sync(
            config.agent.agents_dir.clone(),
            || -> Result<()> {
                for (name, expected) in input["agent_manifests"].as_object().into_iter().flatten() {
                    let actual = opencoder_core::brain::resources::agent_manifest(name)
                        .map_err(anyhow::Error::msg)?;
                    ensure!(
                        expected.as_str() == Some(actual.as_str()),
                        "pinned resource mismatch for {name}"
                    );
                }
                Ok(())
            },
        );
        return Ok(match matches {
            Ok(()) => RpcReply::ok(
                json!({"compatible":true,"features":["dag_dynamic_v1","brain_scheduler_v4"]}),
            ),
            Err(error) => RpcReply::error(412, error.to_string()),
        });
    }
    if action == "notice_ack" {
        return super::outbox::ack(worker, reference, input).await;
    }
    if matches!(action, "layered_output" | "layered_summary") {
        return super::v4::output::query(worker, reference, action, input).await;
    }
    let current = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(&reference.id)
        .is_some_and(|record| record.assignment.request.input["schema_version"] == 4);
    if !current {
        return Ok(RpcReply::error(409, "unsupported brain schema; expected 4"));
    }
    super::v4::handle(worker, reference, action, input).await
}
