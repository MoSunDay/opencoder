//! Keep the admission marker immutable, including for legacy parents.
use crate::journal::Record;
use opencoder_core::{brain::execution_io_version, fleet::ExecutionKind};
use serde_json::json;

pub(super) fn annotate(record: &mut Record) -> anyhow::Result<()> {
    let input = &record.assignment.request.input;
    let version = execution_io_version(input)?;
    let standalone_dag = record.assignment.index.kind == ExecutionKind::Dag
        && !crate::brain::v4::parent::leaf(input);
    if version != 0 || standalone_dag {
        if !record.annotations.is_object() {
            record.annotations = json!({});
        }
        record.annotations["execution_io_version"] = json!(version.max(u32::from(standalone_dag)));
    }
    Ok(())
}
