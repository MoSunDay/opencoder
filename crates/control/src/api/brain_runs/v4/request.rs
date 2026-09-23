use crate::AppState;
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::layered::*, brain::*};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedRequest {
    schema_version: u32,
    plan: PlanRef,
    #[serde(default)]
    inputs: BTreeMap<String, Value>,
    #[serde(default)]
    artifacts: BTreeMap<String, ArtifactRef>,
    #[serde(default)]
    parent: Option<LayeredParent>,
    #[serde(default)]
    depth: u32,
}

pub async fn resolve(
    state: &Arc<AppState>,
    value: &Value,
) -> Result<(LayeredRequest, Vec<BrainCapabilityDescriptor>)> {
    let child_id = value["id"].as_str().map(str::to_owned);
    let mut value = value.clone();
    {
        let object = value
            .as_object_mut()
            .context("run request must be an object")?;
        object.remove("id");
        object.remove("node_id");
    }
    // Control freezes the request verbatim, so a caller may post that shape
    // back. The envelope keys are ignored because the node only reads the
    // `layered_request` object, which is the one authoritative request.
    let frozen = ["layered_request", "request"]
        .iter()
        .filter_map(|key| value.get(*key))
        .find(|frozen| frozen["schema_version"] == LAYERED_SCHEMA_VERSION)
        .cloned();
    let inline = value.get("plan").is_some_and(|plan| {
        plan["schema_version"].as_u64() == Some(u64::from(LAYERED_SCHEMA_VERSION))
    });
    let mut request: LayeredRequest = match frozen {
        Some(frozen) => serde_json::from_value(frozen)?,
        None if inline => serde_json::from_value(value)?,
        None if value.get("plan").is_some() => {
            let saved: SavedRequest = serde_json::from_value(value)?;
            ensure!(
                saved.schema_version == LAYERED_SCHEMA_VERSION,
                "{LAYERED_MIGRATION}"
            );
            let version = state
                .fleet
                .brain_plan_document(&saved.plan.id, saved.plan.version)
                .await?
                .context("plan version not found")?;
            ensure!(
                version.plan["schema_version"] == LAYERED_SCHEMA_VERSION,
                "unsupported plan version; expected schema 6"
            );
            let plan: LayeredPlan = serde_json::from_value(version.plan)?;
            opencoder_brain::layered::validate_plan(&plan)?;
            let mut inputs = plan.inputs.clone();
            inputs.extend(saved.inputs);
            LayeredRequest {
                schema_version: LAYERED_SCHEMA_VERSION,
                plan,
                inputs,
                artifacts: saved.artifacts,
                origin: Some(LayeredOrigin {
                    plan_id: saved.plan.id,
                    version: saved.plan.version,
                }),
                parent: saved.parent,
                depth: saved.depth,
            }
        }
        None => serde_json::from_value(value)?,
    };
    let mut inputs = request.plan.inputs.clone();
    inputs.extend(request.inputs);
    request.inputs = inputs;
    opencoder_core::brain::pc_issue::validate_plan(&request.plan)?;
    if opencoder_core::brain::pc_issue::is_plan(&request.plan) {
        let problem = request
            .inputs
            .get("problem")
            .context("problem input required")?;
        super::super::attachments::images(state, problem).await?;
        let settings = request
            .inputs
            .get("settings")
            .context("PC issue settings required")?;
        for key in ["workspace", "helper", "device_node", "build_node"] {
            ensure!(
                settings[key].as_str().is_some_and(|s| !s.trim().is_empty()),
                "PC issue setting {key} required"
            );
        }
        ensure!(
            settings["max_repair_rounds"]
                .as_u64()
                .is_some_and(|n| (1..=2).contains(&n)),
            "PC repair rounds must be 1 or 2"
        );
        ensure!(
            settings["max_repair_rounds"].as_u64() == Some(u64::from(request.plan.max_rounds)),
            "PC repair budget must match plan.max_rounds"
        );
    }
    opencoder_brain::layered::validate_request(&request)?;
    if let Some(parent) = &request.parent {
        let snapshot = super::read::snapshot(state, &parent.run_id)
            .await
            .map_err(|reply| anyhow::anyhow!("parent run unavailable: {}", reply.body))?;
        ensure!(!snapshot.run.phase.terminal(), "parent run is terminal");
        ensure!(
            request.depth == snapshot.run.depth + 1,
            "parent depth mismatch"
        );
        let operation = snapshot
            .operations
            .iter()
            .find(|op| op.operation_id == parent.operation_id)
            .context("parent operation missing")?;
        ensure!(
            operation.execution_kind == opencoder_core::fleet::ExecutionKind::Brain
                && child_id.as_deref() == Some(operation.execution_id.as_str())
                && operation.node_id == parent.node_id
                && operation.layer == parent.layer
                && operation.status == LayeredOperationStatus::Creating
                && !operation.cancel_requested,
            "parent operation identity mismatch"
        );
        let origin = request
            .origin
            .as_ref()
            .context("nested plan must name its saved version")?;
        ensure!(
            operation.capability_id == format!("plan-{}@{}", origin.plan_id, origin.version),
            "nested capability version mismatch"
        );
        let saved = state
            .fleet
            .brain_plan_document(&origin.plan_id, origin.version)
            .await?
            .context("nested saved version missing")?;
        ensure!(
            serde_json::to_value(&request.plan)?
                == serde_json::to_value(serde_json::from_value::<LayeredPlan>(saved.plan)?)?,
            "nested plan does not match its saved version"
        );
    }
    let capabilities = super::catalog::available(state, &request).await?;
    Ok((request, capabilities))
}
