//! Frozen model capacity and cumulative human-input admission.
use crate::{journal::Record, Worker};
use anyhow::{Context, Result};
use opencoder_brain::layered::budget::{self, ContextBudget};
use opencoder_core::{
    brain::{layered::*, BrainCapabilityDescriptor},
    Config,
};
use serde_json::Value;

pub(crate) fn admit(input: &Value, config: &Config) -> Result<ContextBudget> {
    let request: LayeredRequest = serde_json::from_value(input["layered_request"].clone())?;
    let capabilities: Vec<BrainCapabilityDescriptor> =
        serde_json::from_value(input["frozen_capabilities"].clone())
            .context("frozen capability descriptors required for complete-evidence admission")?;
    budget::admission_budget(&request, &capabilities, config.context_limit())
}

pub(super) fn frozen(worker: &Worker, record: &Record) -> Result<ContextBudget> {
    if let Some(value) = record.annotations.get("brain_context_budget") {
        return Ok(serde_json::from_value(value.clone())?);
    }
    let config = record
        .queue
        .as_ref()
        .map(|queue| Ok(queue.config.clone()))
        .unwrap_or_else(|| worker.configuration())?;
    admit(&record.assignment.request.input, &config)
}

pub(super) async fn human(worker: &Worker, record: &Record, text: &str) -> Result<()> {
    let capacity = frozen(worker, record)?;
    let mut bytes = if text.is_empty() {
        0
    } else {
        budget::human_input_bytes(text)?
    };
    let mut after = 0;
    loop {
        let events = worker
            .inner
            .state
            .store
            .brain_layered_events(&record.assignment.index.id, after, 500)
            .await?;
        for event in &events {
            if let Some(text) = &event.user_input {
                bytes += budget::human_input_bytes(text)?;
            }
        }
        capacity.check_guidance(bytes)?;
        if events.len() < 500 {
            return Ok(());
        }
        after = events.last().unwrap().seq;
    }
}

pub(super) fn context(worker: &Worker, record: &Record, context: &LayeredContext) -> Result<()> {
    let capacity = frozen(worker, record)?;
    budget::validate_context(context, capacity.context_limit)
}
