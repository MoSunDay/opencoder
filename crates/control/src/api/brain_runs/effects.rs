//! Durable node outbox delivery. The source keeps replaying until the root
//! commits the receipt, then an acknowledgement is sent back to the source.
use super::{catalog, plans, runs};
use crate::AppState;
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::*, fleet::*};
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn deliver(
    state: Arc<AppState>,
    node: String,
    execution: ExecutionRef,
    action: String,
    input: Value,
) -> Result<()> {
    let _process_lock = state
        .fleet
        .request_lock("brain-control", &execution.id)
        .await?;
    let index = state
        .fleet
        .index(&execution.id)
        .await?
        .context("source execution is not registered")?;
    ensure!(
        index.node_id == node && index.kind == execution.kind,
        "outbox source ownership mismatch"
    );
    match action.as_str() {
        "notice" => notice(&state, &node, &execution, input).await,
        "publish" => {
            ensure!(
                execution.kind == ExecutionKind::Brain,
                "only brain roots publish plans"
            );
            let _gate = state.brain_gate.lock(&execution.id).await;
            let mut version: PlanVersion = serde_json::from_value(input)?;
            // All definitions must already be pinned in the candidate. Pin
            // missing definitions before returning the exact publication.
            if let Some(saved) = state
                .fleet
                .brain_plan_version(&version.id, version.version)
                .await?
            {
                version = saved;
            } else {
                if let Err(error) = plans::pin(&state, &mut version.plan).await {
                    let reply = runs::call(
                        &state,
                        &execution.id,
                        "publication_failed",
                        json!({"error":format!("plan capability validation: {error:#}")}),
                    )
                    .await;
                    ensure!(
                        reply.status < 300,
                        "publication failure receipt was not committed"
                    );
                    return Ok(());
                }
                state.fleet.save_brain_plan(&version).await?;
            }
            let reply = runs::call(&state, &execution.id, "published", json!(version)).await;
            ensure!(reply.status < 300, "plan adoption: {}", reply.body);
            Ok(())
        }
        "action" => {
            ensure!(
                execution.kind == ExecutionKind::Brain,
                "only brain roots dispatch"
            );
            let receipt: ActionReceipt = serde_json::from_value(input)?;
            let _gate = state.brain_gate.lock(&execution.id).await;
            let authorized = runs::call(&state, &execution.id, "authorize", json!(receipt)).await;
            if authorized.status >= 300 {
                return Ok(());
            }
            let reply = match receipt.kind {
                ActionKind::Execute => {
                    let request: CreateExecution = serde_json::from_value(receipt.request.clone())?;
                    let resources: Vec<ResourceUse> =
                        serde_json::from_value(request.input["_brain"]["resources"].clone())?;
                    if !state
                        .fleet
                        .claim_brain_resources(&request.id, &execution.id, &resources)
                        .await?
                    {
                        let ack = runs::call(&state, &execution.id, "receipt", json!({"id":receipt.id,"reply":RpcReply::error(423,"waiting for conflicting shared resources")})).await;
                        ensure!(ack.status < 300, "resource wait receipt was not committed");
                        return Ok(());
                    }
                    crate::api::executions::submit(&state, request).await
                }
                ActionKind::Cancel => {
                    let child: ExecutionRef = serde_json::from_value(receipt.request.clone())?;
                    if let Some(index) = state.fleet.index(&child.id).await? {
                        state
                            .hub
                            .call(
                                &index.node_id,
                                NodeOperation::Command {
                                    execution: child,
                                    command: ExecutionCommand {
                                        action: "cancel".into(),
                                        input: Value::Null,
                                    },
                                },
                            )
                            .await
                    } else {
                        let notice = BrainNotice {
                            parent: BrainLink {
                                run_id: execution.id.clone(),
                                node_id: node.clone(),
                                instance_id: receipt.instance_id.clone(),
                                attempt: receipt.attempt,
                            },
                            execution: child.clone(),
                            node_id: node.clone(),
                            sequence: 1,
                            status: StepStatus::Cancelled,
                            output: None,
                            error: Some("cancelled before admission".into()),
                            at_ms: opencoder_core::message::now_ms(),
                        };
                        let reply =
                            runs::call(&state, &execution.id, "notice", json!(notice)).await;
                        state.fleet.release_brain_resources(&child.id).await?;
                        reply
                    }
                }
            };
            if receipt.kind == ActionKind::Execute
                && (400..500).contains(&reply.status)
                && !matches!(reply.status, 408 | 409 | 423 | 429)
            {
                state.fleet.release_brain_resources(&receipt.id).await?;
            }
            let ack = runs::call(
                &state,
                &execution.id,
                "receipt",
                json!({"id":receipt.id,"reply":reply}),
            )
            .await;
            ensure!(ack.status < 300, "action receipt delivery: {}", ack.body);
            Ok(())
        }
        _ => anyhow::bail!("unknown brain outbox message"),
    }
}

async fn notice(
    state: &Arc<AppState>,
    node: &str,
    execution: &ExecutionRef,
    input: Value,
) -> Result<()> {
    let notice: BrainNotice = serde_json::from_value(input.clone())?;
    ensure!(
        notice.node_id == node && &notice.execution == execution,
        "notice source mismatch"
    );
    let root = state
        .fleet
        .index(&notice.parent.run_id)
        .await?
        .context("parent root not found")?;
    ensure!(
        root.kind == ExecutionKind::Brain && root.node_id == notice.parent.node_id,
        "parent owner mismatch"
    );
    let capability = input["capability_id"].as_str().map(str::to_owned);
    let reply = runs::call(state, &root.id, "notice", input).await;
    ensure!(
        reply.status < 300,
        "root receipt was not committed: {}",
        reply.body
    );
    if notice.status.terminal() {
        if let Some(capability) = capability {
            catalog::record_evidence(state, &capability, &notice).await?;
        }
        for run in state.fleet.release_brain_resources(&execution.id).await? {
            let reply = runs::call(
                state,
                &run,
                "wake",
                json!({"resource_released_by":execution.id}),
            )
            .await;
            ensure!(reply.status < 300, "resource waiter wake not committed");
        }
    }
    let ack = state
        .hub
        .call(
            node,
            NodeOperation::Brain {
                execution: execution.clone(),
                action: "notice_ack".into(),
                input: json!({"sequence":notice.sequence}),
            },
        )
        .await;
    ensure!(ack.status < 300, "source notice acknowledgement failed");
    Ok(())
}
