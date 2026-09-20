use crate::AppState;
use anyhow::{ensure, Context, Result};
use opencoder_core::brain::*;
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
}

pub async fn resolve(
    state: &Arc<AppState>,
    value: &Value,
) -> Result<(BrainSchedulerRequest, Vec<BrainCapabilityDescriptor>)> {
    let mut value = value.clone();
    let object = value
        .as_object_mut()
        .context("run request must be an object")?;
    object.remove("id");
    object.remove("node_id");
    let request = if object.contains_key("plan") {
        let saved: SavedRequest = serde_json::from_value(value)?;
        ensure!(saved.schema_version == 3, "{SCHEDULER_MIGRATION}");
        let version = state
            .fleet
            .brain_plan_document(&saved.plan.id, saved.plan.version)
            .await?
            .context("plan version not found")?;
        ensure!(
            version.plan["schema_version"] == 3,
            "historical plans are read-only; create a scheduler plan"
        );
        let plan: SchedulerPlan = serde_json::from_value(version.plan)?;
        opencoder_brain::scheduler::validate_plan(&plan)?;
        plan.request(saved.inputs)
    } else {
        serde_json::from_value(value)?
    };
    opencoder_brain::scheduler::validate_request(&request)?;
    let capabilities = super::catalog::available(state, &request).await?;
    Ok((request, capabilities))
}
