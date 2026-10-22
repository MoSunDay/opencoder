use super::*;
use anyhow::Context;

pub(crate) async fn root_ack(
    worker: &Worker,
    reference: &ExecutionRef,
    action: &str,
    input: Value,
) -> Result<RpcReply> {
    {
        let journal = worker.inner.journal.lock().await;
        let record = journal
            .summaries
            .get(&reference.id)
            .context("root execution missing")?;
        ensure!(
            record.index.kind == reference.kind,
            "root execution kind mismatch"
        );
    }
    let id = &reference.id;
    let state = worker
        .inner
        .state
        .store
        .report_run_state(id)
        .await?
        .context("layered run missing")?;
    let (action, item, cursor, reply) = match action {
        "layered_wake_ack" => {
            let generation = input["generation"]
                .as_u64()
                .context("generation required")?;
            if generation > state.generation {
                return Ok(RpcReply::ok(json!({"acknowledged":generation})));
            }
            ("layered_wake", String::new(), generation, json!(generation))
        }
        "layered_guidance_ack" => {
            let seq = input["seq"].as_u64().context("guidance seq required")?;
            ensure!(
                seq <= state.last_event_seq,
                "guidance acknowledgement exceeds event watermark"
            );
            ("layered_guidance", String::new(), seq, json!(seq))
        }
        "layered_dispatch_ack" => {
            let operation = input["operation_id"]
                .as_str()
                .context("operation_id required")?;
            ensure!(
                worker
                    .inner
                    .state
                    .store
                    .report_operation(id, operation)
                    .await?
                    .is_some(),
                "unknown dispatch acknowledgement"
            );
            ("layered_dispatch", operation.into(), 1, json!(operation))
        }
        "layered_cancel_ack" => {
            let op: opencoder_core::brain::layered::LayeredOperation =
                serde_json::from_value(input)?;
            let current = worker
                .inner
                .state
                .store
                .report_operation(id, &op.operation_id)
                .await?
                .context("unknown cancellation")?;
            ensure!(current.cancel_requested, "unknown cancellation");
            (
                "layered_cancel",
                op.operation_id.clone(),
                1,
                json!(op.operation_id),
            )
        }
        _ => anyhow::bail!("unknown report acknowledgement"),
    };
    let ack = ReportAck {
        execution_id: id.clone(),
        action: action.into(),
        item,
        cursor,
    };
    let mut journal = worker.inner.journal.lock().await;
    save_ack(&worker.inner.state.store, &mut journal, &ack).await?;
    Ok(RpcReply::ok(json!({"acknowledged":reply})))
}
