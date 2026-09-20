//! One catalog adapter shared by admission, saved plans and each activation.
use crate::{api::brain_runs::catalog, AppState};
use anyhow::{ensure, Result};
use opencoder_core::brain::*;
use serde_json::Value;
use std::sync::Arc;

pub fn descriptors(raw: Vec<Value>) -> Vec<BrainCapabilityDescriptor> {
    raw.into_iter()
        .filter_map(|value| {
            Some(BrainCapabilityDescriptor {
                capability_id: value
                    .get("id")
                    .or_else(|| value.get("capability_id"))?
                    .as_str()?
                    .into(),
                kind: serde_json::from_value(value.get("kind")?.clone()).ok()?,
                target: value["target"].as_str().unwrap_or("").into(),
                input_desc: value["input_desc"].as_str().unwrap_or("").into(),
                output_desc: value["output_desc"].as_str().unwrap_or("").into(),
                required_inputs: value["required_inputs"]
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                definition: value.get("definition").cloned().unwrap_or(Value::Null),
                version: value["version"].as_str().unwrap_or("").into(),
            })
        })
        .collect()
}

pub async fn available(
    state: &Arc<AppState>,
    request: &BrainSchedulerRequest,
) -> Result<Vec<BrainCapabilityDescriptor>> {
    let all = descriptors(catalog::capabilities(state).await?);
    let capabilities = opencoder_brain::scheduler::prefilter(&all, request);
    for id in &request.capability_ids {
        ensure!(
            capabilities
                .iter()
                .filter(|c| &c.capability_id == id)
                .count()
                == 1,
            "selected capability unavailable or ambiguous: {id}"
        );
    }
    ensure!(!capabilities.is_empty(), "no available capabilities");
    Ok(capabilities)
}
