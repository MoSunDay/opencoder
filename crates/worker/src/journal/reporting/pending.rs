use super::*;
use opencoder_core::brain::layered::{LayeredDispatchIntent, LayeredOperation};
use std::ops::Bound::{Excluded, Included, Unbounded};

/// Keep at most one pending root, reusing retries within its intent generation.
/// A different root/generation replaces it; an empty dispatch queue releases it.
pub(crate) struct DispatchCache {
    id: String,
    generation: u64,
    intent: LayeredDispatchIntent,
}

pub(super) fn needs_terminal(record: &Record) -> bool {
    if !record.assignment.index.status.terminal()
        || !parent::reports(&record.assignment.request.input)
    {
        return false;
    }
    match parent::reporting(&record.assignment.request.input).and_then(|binding| {
        binding
            .map(|b| parent::terminal(record, &b))
            .transpose()
            .map(Option::flatten)
    }) {
        Ok(Some(terminal)) => {
            record.annotations[parent::ACK].as_u64().unwrap_or(0) < terminal.source_sequence
        }
        Ok(None) => false,
        Err(_) => true,
    }
}
impl Journal {
    pub(crate) fn terminal_ack_sequence(&self, reference: &ExecutionRef) -> Result<Option<u64>> {
        let Some(summary) = self.summaries.get(&reference.id) else {
            return Ok(None);
        };
        anyhow::ensure!(
            summary.index.kind == reference.kind,
            "source execution kind mismatch"
        );
        let record = self
            .records
            .get(&reference.id)
            .ok_or_else(|| anyhow::anyhow!("execution has no layered parent"))?;
        anyhow::ensure!(
            parent::reports(&record.assignment.request.input),
            "execution has no layered parent"
        );
        Ok(Some(
            record.events.last().and_then(|e| e.seq).unwrap_or(1).max(1) as u64,
        ))
    }
    pub(super) fn update_terminal_candidate(&mut self, record: &Record) {
        if needs_terminal(record) {
            self.terminal_candidates
                .insert(record.assignment.index.id.clone());
        } else {
            self.terminal_candidates.remove(&record.assignment.index.id);
        }
    }
    pub(crate) fn has_terminal_reports(&self) -> bool {
        !self.terminal_candidates.is_empty()
    }
    pub(crate) fn terminal_frames(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<NodeFrame>> {
        let after = after.unwrap_or("");
        let ids = self
            .terminal_candidates
            .range::<str, _>((Excluded(after), Unbounded))
            .chain(
                self.terminal_candidates
                    .range::<str, _>((Unbounded, Included(after))),
            );
        let mut frames = Vec::new();
        for id in ids.take(limit) {
            let record = &self.records[id];
            if let Some(binding) = parent::reporting(&record.assignment.request.input)? {
                if let Some(terminal) = parent::terminal(record, &binding)? {
                    frames.push(NodeFrame::Brain {
                        execution: record.assignment.index.execution_ref(),
                        action: "layered_terminal".into(),
                        input: json!(terminal),
                    });
                }
            }
        }
        Ok(frames)
    }
    pub(crate) async fn dispatch_payload(
        &self,
        id: &str,
        operation: &LayeredOperation,
        cache: &mut Option<DispatchCache>,
    ) -> Result<Value> {
        let generation = self
            .summaries
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("root execution missing"))?
            .intent_generation
            .ok_or_else(|| anyhow::anyhow!("dispatch intent generation missing"))?;
        if !cache
            .as_ref()
            .is_some_and(|c| c.id == id && c.generation == generation)
        {
            // Drop the previous root before materializing another large intent.
            *cache = None;
            let mut record = self
                .get(id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("root execution missing"))?;
            let value = record
                .annotations
                .as_object_mut()
                .and_then(|a| a.remove("layered_intent"))
                .ok_or_else(|| anyhow::anyhow!("dispatch intent missing"))?;
            *cache = Some(DispatchCache {
                id: id.into(),
                generation,
                intent: serde_json::from_value(value)?,
            });
        }
        let intent = &cache.as_ref().expect("dispatch cache populated").intent;
        anyhow::ensure!(
            intent
                .operations
                .iter()
                .any(|op| op.operation_id == operation.operation_id
                    && op.activation == operation.activation),
            "dispatch intent no longer owns operation"
        );
        let assignment = intent
            .assignments
            .iter()
            .find(|a| a.node_id == operation.node_id && a.capability_id == operation.capability_id)
            .ok_or_else(|| anyhow::anyhow!("dispatch assignment missing"))?;
        let capability = intent
            .capabilities
            .iter()
            .find(|c| c.capability_id == operation.capability_id)
            .ok_or_else(|| anyhow::anyhow!("dispatch capability missing"))?;
        Ok(json!({"operation":operation,"assignment":assignment,"capability":capability}))
    }
}
