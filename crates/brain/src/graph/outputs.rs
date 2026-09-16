use anyhow::{ensure, Context, Result};
use opencoder_core::brain::*;
use serde_json::json;

pub fn output_schema() -> DataSchema {
    serde_json::from_value(json!({"type":"object"})).expect("constant schema")
}

pub fn capture(run: &mut BrainRun, visit: &str) -> Result<()> {
    let instance = &run.instances[visit];
    let plan = &run.request.plan.as_ref().context("plan missing")?.plan;
    let definition = plan
        .instances
        .iter()
        .find(|i| i.id == instance.step_id)
        .context("unknown instance")?;
    let envelope = instance
        .output
        .as_ref()
        .context("execution has no output envelope")?;
    let values = envelope
        .value
        .as_object()
        .context("capability must return named outputs as an object")?;
    let round = run.expansions[&instance.step_id]
        .iter()
        .position(|id| id == visit)
        .context("missing round")? as u32
        + 1;
    let mut records = vec![];
    for (name, value) in values {
        ensure!(
            definition.outputs.contains(name),
            "undeclared output {name}"
        );
        let mut value: ProducedOutput =
            serde_json::from_value(value.clone()).with_context(|| format!("output {name}"))?;
        for assessment in [&mut value.completion, &mut value.verification] {
            assessment.evidence.retain(|s| !s.trim().is_empty());
            if assessment.evidence.is_empty() {
                assessment.passed = None;
            }
        }
        value.artifacts.extend(envelope.artifacts.clone());
        records.push(OutputRecord {
            id: format!("{visit}/{name}"),
            output: name.clone(),
            visit: visit.into(),
            round,
            value,
        });
    }
    for record in records {
        run.graph.outputs.entry(record.id.clone()).or_insert(record);
    }
    Ok(())
}
