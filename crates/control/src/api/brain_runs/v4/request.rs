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
    let request = match frozen {
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
                "historical plans are read-only; create a layered plan"
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
    opencoder_brain::layered::validate_request(&request)?;
    let capabilities = super::catalog::available(state, &request).await?;
    Ok((request, capabilities))
}
