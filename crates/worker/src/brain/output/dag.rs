use super::artifacts;
use crate::{journal::Record, Worker};
use anyhow::{Context, Result};
use opencoder_core::brain::ArtifactRef;
use opencoder_dag::StepKind;
use serde_json::Value;

pub(super) fn output(_worker: &Worker, record: &Record) -> Result<(Value, Vec<ArtifactRef>)> {
    let id = &record.assignment.index.id;
    let parent = crate::layout::dag::accepted_parent(record)?;
    let definition = record
        .assignment
        .definition
        .as_ref()
        .context("DAG definition missing")?;
    let spec = opencoder_dag::decode_spec(definition.get("spec").unwrap_or(definition))
        .map_err(anyhow::Error::msg)?;
    let execution = record.assignment.index.execution_ref();
    let mut outputs = serde_json::Map::new();
    let mut files = Vec::new();
    for step in &spec.steps {
        let dir = opencoder_dag::artifacts::execution_dir(&parent, id, &step.name, None)
            .map_err(anyhow::Error::msg)?;
        let structured = artifacts::json(&dir.join("output.json"))?;
        let value = if structured.is_null() {
            super::super::workspace::checked(&dir, &dir.join("output.txt"))?;
            Value::String(std::fs::read_to_string(dir.join("output.txt"))?)
        } else {
            structured
        };
        outputs.insert(step.name.clone(), value);
        files.extend(artifacts::directory(
            &execution, &dir, &step.name, None, true,
        )?);
        if matches!(step.kind, StepKind::Dynamic { .. }) {
            let items = artifacts::json(&dir.join("instances.json"))?;
            for index in 0..items
                .as_array()
                .context("DAG instance manifest must be an array")?
                .len()
            {
                let instance =
                    opencoder_dag::artifacts::execution_dir(&parent, id, &step.name, Some(index))
                        .map_err(anyhow::Error::msg)?;
                files.extend(artifacts::directory(
                    &execution,
                    &instance,
                    &step.name,
                    Some(index),
                    true,
                )?);
            }
        }
    }
    Ok((Value::Object(outputs), files))
}
