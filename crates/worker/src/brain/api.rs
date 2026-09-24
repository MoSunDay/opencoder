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
        if let Err(error) = matches {
            return Ok(RpcReply::error(412, error.to_string()));
        }
        let mut body = json!({"compatible":true,"features":["dag_dynamic_v1","brain_scheduler_v7","brain_pc_issue_v1","pc_candidate_v1",opencoder_core::fleet::private_files::CAPABILITY]});
        if input["private_files"] == true {
            body["image_digest"] = json!(tokio::task::spawn_blocking(
                opencoder_core::fleet::private_files::runtime_image_digest
            )
            .await?
            .map_err(anyhow::Error::msg)?);
        }
        return Ok(RpcReply::ok(body));
    }
    if action == "notice_ack" {
        return super::outbox::ack(worker, reference, input).await;
    }
    if matches!(action, "layered_output" | "layered_summary") {
        return super::v4::output::query(worker, reference, action, input).await;
    }
    let schema = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(&reference.id)
        .and_then(|record| record.assignment.request.input["schema_version"].as_u64());
    if matches!(schema, Some(4 | 5 | 6)) {
        return super::v4::history::read(worker, reference, action, input, schema.unwrap() as u32)
            .await;
    }
    if schema != Some(7) {
        return Ok(RpcReply::error(409, "unsupported brain schema; expected 7"));
    }
    super::v4::handle(worker, reference, action, input).await
}
