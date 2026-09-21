//! Saved immutable plans participate in the ordinary capability catalog.
use crate::AppState;
use anyhow::{ensure, Context, Result};
use opencoder_core::brain::{layered::*, BrainCapabilityDescriptor};
use opencoder_core::fleet::ExecutionKind;
use serde_json::{json, Value};
use std::{collections::BTreeSet, sync::Arc};

pub async fn list(state: &Arc<AppState>) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for definition in state.fleet.definitions("brain_plan").await? {
        let id = definition["id"].as_str().context("plan id missing")?;
        let mut before = None;
        loop {
            let versions = state.fleet.brain_plan_documents(id, before).await?;
            if versions.is_empty() {
                break;
            }
            before = versions.last().map(|v| v.version);
            for version in versions {
                if version.plan["schema_version"] != LAYERED_SCHEMA_VERSION {
                    continue;
                }
                let plan: LayeredPlan = serde_json::from_value(version.plan)?;
                result.push(json!({
                    "id":format!("plan-{id}@{}", version.version), "kind":"brain",
                    "target":format!("{id}@{}", version.version), "summary":plan.title,
                    "input_desc":"Named inputs for this saved plan", "output_desc":"Completed plan result and evidence",
                    "required_inputs":[], "version":version.version.to_string(), "maturity":"stable",
                    "definition":{"plan_id":id, "version":version.version, "plan":plan}
                }));
            }
        }
    }
    Ok(result)
}

/// Validate the complete reference graph before admitting any root execution.
pub fn validate(
    plan: &LayeredPlan,
    depth: u32,
    catalog: &[BrainCapabilityDescriptor],
) -> Result<()> {
    fn visit(
        plan: &LayeredPlan,
        depth: u32,
        catalog: &[BrainCapabilityDescriptor],
        path: &mut BTreeSet<String>,
    ) -> Result<()> {
        ensure!(depth <= LAYERED_MAX_DEPTH, "nesting depth exceeded");
        opencoder_brain::layered::validate_plan(plan)?;
        for node in &plan.nodes {
            let cap = catalog
                .iter()
                .find(|cap| cap.capability_id == node.capability_id)
                .with_context(|| format!("node capability unavailable: {}", node.capability_id))?;
            ensure!(
                catalog
                    .iter()
                    .filter(|item| item.capability_id == node.capability_id)
                    .count()
                    == 1,
                "ambiguous capability"
            );
            ensure!(
                matches!(
                    cap.kind,
                    ExecutionKind::Brain
                        | ExecutionKind::Agent
                        | ExecutionKind::Operator
                        | ExecutionKind::Dag
                        | ExecutionKind::Todos
                        | ExecutionKind::Team
                ) && !cap.target.trim().is_empty()
                    && !cap.input_desc.trim().is_empty()
                    && !cap.output_desc.trim().is_empty()
                    && cap.definition.is_object()
                    && !cap.version.trim().is_empty(),
                "unavailable capability: {}",
                cap.capability_id
            );
            if cap.kind != ExecutionKind::Brain {
                continue;
            }
            ensure!(
                path.insert(cap.capability_id.clone()),
                "cyclic plan capability: {}",
                cap.capability_id
            );
            let nested: LayeredPlan = serde_json::from_value(cap.definition["plan"].clone())?;
            visit(&nested, depth + 1, catalog, path)?;
            path.remove(&cap.capability_id);
        }
        Ok(())
    }
    visit(plan, depth, catalog, &mut BTreeSet::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan(cap: &str) -> LayeredPlan {
        serde_json::from_value(json!({"schema_version":4,"title":"nested","objective":"verify nesting", "nodes":[{"node_id":"step","title":"perform the plan","capability_id":cap}]})).unwrap()
    }
    fn cap(id: &str, child: Option<LayeredPlan>) -> BrainCapabilityDescriptor {
        BrainCapabilityDescriptor {
            capability_id: id.into(),
            kind: if child.is_some() {
                ExecutionKind::Brain
            } else {
                ExecutionKind::Agent
            },
            target: id.into(),
            input_desc: "inputs".into(),
            output_desc: "result".into(),
            required_inputs: vec![],
            definition: child.map(|plan| json!({"plan":plan})).unwrap_or(json!({})),
            version: "1".into(),
        }
    }
    #[test]
    fn nested_plans_validate_every_reference() {
        let catalog = vec![cap("child", Some(plan("leaf"))), cap("leaf", None)];
        validate(&plan("child"), 0, &catalog).unwrap();
        assert!(validate(&plan("child"), 0, &catalog[..1])
            .unwrap_err()
            .to_string()
            .contains("unavailable"));
    }
    #[test]
    fn cyclic_and_overdeep_plans_fail_before_dispatch() {
        let catalog = vec![cap("child", Some(plan("child")))];
        assert!(validate(&plan("child"), 0, &catalog)
            .unwrap_err()
            .to_string()
            .contains("cyclic"));
        let catalog = vec![cap("child", Some(plan("leaf"))), cap("leaf", None)];
        assert!(validate(&plan("child"), 3, &catalog)
            .unwrap_err()
            .to_string()
            .contains("depth"));
    }
}
