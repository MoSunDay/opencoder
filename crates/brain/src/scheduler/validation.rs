use anyhow::{ensure, Result};
use opencoder_core::{brain::*, fleet::ExecutionKind};
use std::collections::BTreeSet;

pub fn validate_request(r: &BrainSchedulerRequest) -> Result<()> {
    ensure!(r.schema_version == 3, "{SCHEDULER_MIGRATION}");
    ensure!(
        !r.objective.trim().is_empty() && r.objective.len() <= 65536,
        "objective required (maximum 64 KiB)"
    );
    ensure!(
        (1..=256).contains(&r.max_rounds),
        "max_rounds must be 1..256"
    );
    ensure!(
        r.inputs
            .keys()
            .all(|n| !n.trim().is_empty() && n.len() <= 128),
        "invalid named input"
    );
    ensure!(
        r.capability_ids.iter().collect::<BTreeSet<_>>().len() == r.capability_ids.len(),
        "duplicate allowed capability"
    );
    Ok(())
}
/// Catalog adapters resolve actual targets first. Missing descriptions or
/// definitions never produce a generic substitute.
pub fn prefilter(
    catalog: &[BrainCapabilityDescriptor],
    request: &BrainSchedulerRequest,
) -> Vec<BrainCapabilityDescriptor> {
    catalog
        .iter()
        .filter(|c| {
            matches!(
                c.kind,
                ExecutionKind::Agent
                    | ExecutionKind::Dag
                    | ExecutionKind::Team
                    | ExecutionKind::Todos
                    | ExecutionKind::Operator
            ) && !c.target.trim().is_empty()
                && !c.input_desc.trim().is_empty()
                && !c.output_desc.trim().is_empty()
                && c.definition.is_object()
                && !c.version.trim().is_empty()
                && (request.capability_ids.is_empty()
                    || request.capability_ids.contains(&c.capability_id))
        })
        .cloned()
        .collect()
}
pub(super) fn reason(s: &str) -> Result<()> {
    ensure!(
        !s.trim().is_empty() && s.len() <= 4096,
        "reason must contain 1..4096 bytes"
    );
    Ok(())
}
pub(super) fn evidence(ids: &[String], ops: &[BrainOperation]) -> Result<()> {
    ensure!(
        ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
        "duplicate evidence"
    );
    ensure!(
        ids.iter().all(|id| ops
            .iter()
            .any(|o| &o.execution_id == id && o.status.successful())),
        "evidence must reference successful terminal executions"
    );
    Ok(())
}
pub(super) fn bindings(
    item: &BrainDispatchItem,
    capability: &BrainCapabilityDescriptor,
    request: &BrainSchedulerRequest,
    ops: &[BrainOperation],
) -> Result<()> {
    ensure!(!item.inputs.is_empty(), "missing input references");
    ensure!(
        capability
            .required_inputs
            .iter()
            .all(|n| item.inputs.contains_key(n)),
        "missing required capability input"
    );
    for (name, binding) in &item.inputs {
        ensure!(!name.trim().is_empty(), "empty binding name");
        match binding {
            BrainInputBinding::Root { name } => ensure!(
                request.inputs.contains_key(name),
                "unknown root input {name}"
            ),
            BrainInputBinding::Execution { execution_id, path } => {
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
            BrainInputBinding::Artifact { reference } => ensure!(
                request.artifacts.contains_key(reference),
                "unknown artifact reference"
            ),
        }
    }
    Ok(())
}
