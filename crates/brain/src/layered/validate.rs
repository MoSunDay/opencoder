//! Save-time and run-time validation of a v4 plan/request. Both the workbench
//! and the control plane call [`validate_plan`], so a stored canvas and an
//! executed canvas accept exactly the same documents.
use super::levels::{layers, validate_shape};
use anyhow::{ensure, Result};
use opencoder_core::brain::layered::*;
use std::collections::BTreeSet;

pub fn validate_plan(plan: &LayeredPlan) -> Result<()> {
    ensure!(
        plan.schema_version == LAYERED_SCHEMA_VERSION,
        "new plans require schema 6; convert the saved version explicitly"
    );
    validate_shape(plan)?;
    ensure!(
        !plan.title.trim().is_empty() && plan.title.chars().count() <= 120,
        "plan title must contain 1..120 characters"
    );
    ensure!(
        !plan.objective.trim().is_empty() && plan.objective.chars().count() <= 4096,
        "plan objective must contain 1..4096 characters"
    );
    ensure!(
        (1..=32).contains(&plan.max_rounds),
        "max_rounds must be 1..32"
    );
    if let Some(todo) = &plan.todo {
        ensure!(
            todo.id.starts_with("pt-") && todo.id.len() <= 64,
            "todo must reference a project todo id"
        );
    }
    let levels = layers(plan)?;
    ensure!(
        levels.len() <= LAYERED_MAX_LAYER_WIDTH,
        "plan depth {} exceeds the dispatch limit",
        levels.len()
    );
    Ok(())
}

pub fn validate_request(request: &LayeredRequest) -> Result<()> {
    ensure!(
        request.schema_version == LAYERED_SCHEMA_VERSION,
        "unsupported run schema"
    );
    ensure!(
        request.plan.schema_version == LAYERED_SCHEMA_VERSION,
        "unsupported plan schema"
    );
    validate_plan(&request.plan)?;
    ensure!(request.depth <= LAYERED_MAX_DEPTH, "nesting depth exceeded");
    ensure!(
        (request.depth > 0) == request.parent.is_some(),
        "nested depth and parent must agree"
    );
    for name in request.plan.inputs.keys() {
        ensure!(!name.trim().is_empty(), "empty plan input name");
    }
    Ok(())
}

pub(crate) fn reason(reason: &str) -> Result<()> {
    ensure!(
        !reason.trim().is_empty() && reason.len() <= 4096,
        "reason must contain 1..4096 bytes"
    );
    Ok(())
}

pub(crate) fn evidence(ids: &[String], ops: &[LayeredOperation]) -> Result<()> {
    ensure!(
        ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
        "duplicate evidence"
    );
    ensure!(
        ids.iter().all(|id| ops
            .iter()
            .any(|o| &o.execution_id == id && o.status.terminal())),
        "evidence must reference terminal executions"
    );
    Ok(())
}

/// Binding rules: required inputs present, references resolvable, and every
/// execution binding points at a successful **ancestor** of the node.
pub(crate) fn bindings(
    assignment: &LayeredAssignment,
    capability: &opencoder_core::brain::BrainCapabilityDescriptor,
    request: &LayeredRequest,
    plan: &LayeredPlan,
    ops: &[LayeredOperation],
) -> Result<()> {
    ensure!(
        capability
            .required_inputs
            .iter()
            .all(|name| assignment.inputs.contains_key(name)),
        "missing required capability input"
    );

    for (name, binding) in &assignment.inputs {
        ensure!(!name.trim().is_empty(), "empty binding name");
        match binding {
            opencoder_core::brain::BrainInputBinding::Value { value } => ensure!(
                serde_json::to_vec(value)?.len() <= 64 * 1024,
                "generated input exceeds 64 KiB"
            ),
            opencoder_core::brain::BrainInputBinding::Root { name } => ensure!(
                request.inputs.contains_key(name) || plan.inputs.contains_key(name),
                "unknown root input {name}"
            ),
            opencoder_core::brain::BrainInputBinding::Execution { execution_id, path } => {
                evidence(std::slice::from_ref(execution_id), ops)?;
                ensure!(
                    path.is_empty() || path.starts_with('/'),
                    "output path must be a JSON pointer"
                );
                ensure!(
                    !path
                        .as_bytes()
                        .windows(2)
                        .any(|w| w[0] == b'~' && w[1] != b'0' && w[1] != b'1')
                        && !path.ends_with('~'),
                    "invalid JSON pointer escape"
                );
            }
            opencoder_core::brain::BrainInputBinding::Artifact { reference } => ensure!(
                request.artifacts.contains_key(reference),
                "unknown artifact reference"
            ),
        }
    }
    Ok(())
}
