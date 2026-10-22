use super::*;

// Validate every record before any recovery writes, without retaining payloads.
pub(super) fn validate(record: &Record) -> Result<()> {
    if record.assignment.index.kind == ExecutionKind::Dag
        && !matches!(
            record.assignment.index.status,
            ExecutionStatus::Done | ExecutionStatus::Error | ExecutionStatus::Cancelled
        )
        && record.annotations.get("dag_parent").is_none()
    {
        bail!("DAG migration blocked by nonterminal execution {}; terminate it with the previous runtime before upgrading", record.assignment.index.id);
    }
    let input = &record.assignment.request.input;
    let legacy = (record.assignment.index.kind == ExecutionKind::Brain
        && input["schema_version"] != opencoder_core::brain::layered::LAYERED_SCHEMA_VERSION)
        || input.get("_brain").is_some()
        || input.get("brain_scheduler").is_some()
        || input.get("brain_receipt").is_some()
        || input.get("playbook_receipt").is_some();
    if legacy
        && !matches!(
            record.assignment.index.status,
            ExecutionStatus::Done | ExecutionStatus::Error | ExecutionStatus::Cancelled
        )
    {
        bail!("brain migration blocked by nonterminal legacy execution {}; let its owning old runtime converge before upgrading", record.assignment.index.id);
    }
    Ok(())
}
