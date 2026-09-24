use crate::{
    api::{error_500, response},
    AppState,
};
use axum::{extract::State, response::Response, Json};
use opencoder_core::{
    brain::{layered::LAYERED_SCHEMA_VERSION, pc_issue},
    fleet::RpcReply,
};
use serde_json::json;
use std::sync::Arc;

/// Cancel only deterministic children carrying this exact stage ownership.
pub(super) async fn cancel_children(
    state: &Arc<AppState>,
    operation: &opencoder_core::brain::layered::LayeredOperation,
) -> anyhow::Result<()> {
    use anyhow::ensure;
    use opencoder_core::fleet::*;
    let Some(stage) = pc_issue::stage(&operation.capability_id) else {
        return Ok(());
    };
    if !matches!(stage, "reproduce" | "verify") {
        return Ok(());
    }
    let suffix = operation
        .run_id
        .strip_prefix("brain-")
        .ok_or_else(|| anyhow::anyhow!("invalid root ID"))?;
    for kind in [ExecutionKind::Dag, ExecutionKind::Team] {
        let id = format!("{}-pc-{suffix}-{stage}-{}", kind.prefix(), operation.round);
        let Some(child) = state.fleet.assignment(&id).await? else {
            continue;
        };
        ensure!(
            child.request.input["pc_issue_parent"]["execution_id"] == operation.execution_id
                && child.request.input["pc_issue_parent"]["run_id"] == operation.run_id,
            "PC child ownership mismatch; refusing cancellation"
        );
        if child.index.status.terminal() {
            continue;
        }
        let reply = state
            .hub
            .call(
                &child.index.node_id,
                NodeOperation::Command {
                    execution: child.index.execution_ref(),
                    command: ExecutionCommand {
                        action: "cancel".into(),
                        input: serde_json::Value::Null,
                    },
                },
            )
            .await;
        ensure!(
            reply.status < 300,
            "PC child cancellation failed: {}",
            reply.body
        );
    }
    Ok(())
}

/// Explicit installation, immutable and idempotent. Upgrade legacy templates by appending.
pub async fn install(State(state): State<Arc<AppState>>) -> Response {
    let previous = match state
        .fleet
        .definition("brain_plan", pc_issue::PLAN_ID)
        .await
    {
        Ok(definition) => definition,
        Err(e) => return error_500(e.to_string()),
    };
    let version = if let Some(definition) = &previous {
        let Some(latest) = definition["latest_version"].as_u64() else {
            return error_500("PC plan latest version missing".into());
        };
        match state
            .fleet
            .brain_plan_document(pc_issue::PLAN_ID, latest)
            .await
        {
            Ok(Some(saved)) if saved.plan["schema_version"] == LAYERED_SCHEMA_VERSION => {
                return response(RpcReply::ok(json!({"definition":definition})));
            }
            Ok(Some(_)) => match latest.checked_add(1) {
                Some(next) => next,
                None => return error_500("PC plan version exhausted".into()),
            },
            Ok(None) => return error_500("PC plan latest document missing".into()),
            Err(e) => return error_500(e.to_string()),
        }
    } else {
        1
    };
    if version > i64::MAX as u64 {
        return error_500("PC plan version exhausted".into());
    }
    let mut plan = pc_issue::plan();
    let nodes = match state.fleet.nodes().await {
        Ok(nodes) => nodes,
        Err(e) => return error_500(e.to_string()),
    };
    for (key, name) in [
        ("device_node", "device-cases"),
        ("build_node", "human-os-02"),
    ] {
        let matches = nodes.iter().filter(|n| n.name == name).collect::<Vec<_>>();
        if matches.len() == 1 {
            plan["inputs"]["settings"][key] = json!(matches[0].id);
        }
    }
    super::plans::save(
        State(state),
        Json(json!({"id":pc_issue::PLAN_ID,"version":version,
        "plan":plan,"changelog":"接入 PC 问题诊断、修复与实证链路", "created_at":0})),
    )
    .await
}

/// Authorize device/build work against the current host-owned stage, under brain-control.
pub(crate) async fn validate_child(
    state: &Arc<AppState>,
    request: &opencoder_core::fleet::CreateExecution,
) -> anyhow::Result<()> {
    use anyhow::{ensure, Context};
    use opencoder_core::fleet::ExecutionKind;
    let owner = &request.input["pc_issue_parent"];
    let root = owner["run_id"].as_str().context("PC parent run required")?;
    let parent = owner["execution_id"]
        .as_str()
        .context("PC parent execution required")?;
    let snapshot = super::v4::read::snapshot(state, root)
        .await
        .map_err(|r| anyhow::anyhow!("PC parent unavailable: {}", r.body))?;
    ensure!(!snapshot.run.phase.terminal(), "PC parent run is terminal");
    let operation = snapshot
        .operations
        .iter()
        .find(|o| o.execution_id == parent)
        .context("PC parent stage missing")?;
    let stage = pc_issue::stage(&operation.capability_id).context("Not a PC stage")?;
    ensure!(
        !operation.cancel_requested && !operation.status.terminal(),
        "PC stage stopped"
    );
    let kind = match (request.kind, request.target.as_deref(), stage) {
        (ExecutionKind::Dag, Some("device-cases"), "reproduce" | "verify") => "dag",
        (ExecutionKind::Team, Some("jy-builder"), "verify") => "team",
        _ => anyhow::bail!("PC child target is outside its stage"),
    };
    let suffix = root.strip_prefix("brain-").context("Invalid PC run ID")?;
    ensure!(
        request.id == format!("{kind}-pc-{suffix}-{stage}-{}", operation.round),
        "PC child identity mismatch"
    );
    let assignment = state
        .fleet
        .assignment(root)
        .await?
        .context("PC root assignment missing")?;
    let settings = &assignment.request.input["layered_request"]["inputs"]["settings"];
    let key = if kind == "dag" {
        "device_node"
    } else {
        "build_node"
    };
    ensure!(
        request.node_id.as_deref() == settings[key].as_str(),
        "PC child node differs from frozen settings"
    );
    Ok(())
}
