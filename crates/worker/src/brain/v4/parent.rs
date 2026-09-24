//! Parent identity of a v4 execution.
//!
//! A leaf child reports exactly one terminal frame to its parent operation. A
//! nested layered run owns its own projection and reports the same way, so a
//! parent never inspects a child body to learn that a node finished.
use crate::journal::Record;
use anyhow::{ensure, Context, Result};
use opencoder_core::{
    brain::layered::{
        LayeredOperationStatus, LayeredParent, LayeredParentTerminal, LayeredSnapshot,
        LayeredTerminalEvent,
    },
    fleet::ExecutionStatus,
};
use serde_json::Value;

/// Journal annotation fence for the single parent terminal frame.
pub const ACK: &str = "brain_layered_ack";

/// The parent binding a reporting execution declares.
#[derive(Clone, Debug)]
pub struct Reporting {
    pub parent: LayeredParent,
    /// A nested run owns a layered projection below the same parent plan.
    pub nested: bool,
}

/// Does this execution report to a parent operation?
pub fn reports(input: &Value) -> bool {
    matches!(input["schema_version"].as_u64(), Some(4..=7))
        && (input["brain_layered"].is_object() || input["layered_request"]["parent"].is_object())
}

/// Leaf child: reports to a parent and owns no layered projection.
pub fn leaf(input: &Value) -> bool {
    matches!(input["schema_version"].as_u64(), Some(4..=7))
        && input["brain_layered"].is_object()
        && !input["layered_request"].is_object()
}

/// Root of a layered projection, either depth 0 or nested below a parent plan.
pub fn root(input: &Value) -> bool {
    matches!(input["schema_version"].as_u64(), Some(4..=7)) && input["layered_request"].is_object()
}

pub fn reporting(input: &Value) -> Result<Option<Reporting>> {
    if !reports(input) {
        return Ok(None);
    }
    if root(input) {
        let parent = input["layered_request"]["parent"].clone();
        if !parent.is_object() {
            // A depth 0 root has no parent to report to.
            return Ok(None);
        }
        return Ok(Some(Reporting {
            parent: serde_json::from_value(parent).context("layered parent binding is invalid")?,
            nested: true,
        }));
    }
    Ok(Some(Reporting {
        parent: binding_parent(&input["brain_layered"])?,
        nested: false,
    }))
}

/// Control writes either a nested `parent` object or the flattened identity.
fn binding_parent(binding: &Value) -> Result<LayeredParent> {
    let value = if binding["parent"].is_object() {
        binding["parent"].clone()
    } else {
        serde_json::json!({
            "run_id": binding["run_id"],
            "operation_id": binding["operation_id"],
            "node_id": binding["node_id"],
            "layer": binding["layer"],
        })
    };
    serde_json::from_value(value).context("layered parent binding is incomplete")
}

/// The parent terminal frame of a finalized child. The node's last event
/// sequence fences the frame so a restarted node cannot replay an old result.
pub fn terminal(record: &Record, reporting: &Reporting) -> Result<Option<LayeredParentTerminal>> {
    let status = match record.assignment.index.status {
        ExecutionStatus::Done => LayeredOperationStatus::Done,
        ExecutionStatus::Error => LayeredOperationStatus::Error,
        ExecutionStatus::Cancelled => LayeredOperationStatus::Cancelled,
        // Interrupted is a recovery state, not a terminal event.
        _ => return Ok(None),
    };
    // A nested run reports its own durable phase; a cancellation that happened
    // before the phase was committed keeps the journal status.
    let status = if reporting.nested {
        match record.result["phase"].as_str() {
            Some("completed") => LayeredOperationStatus::Done,
            Some("failed") => LayeredOperationStatus::Error,
            Some("cancelled") => LayeredOperationStatus::Cancelled,
            _ if status == LayeredOperationStatus::Cancelled => status,
            _ => return Ok(None),
        }
    } else {
        status
    };
    let source_sequence = record
        .events
        .last()
        .and_then(|event| event.seq)
        .unwrap_or(1)
        .max(1) as u64;
    Ok(Some(LayeredParentTerminal {
        run_id: reporting.parent.run_id.clone(),
        operation_id: reporting.parent.operation_id.clone(),
        node_id: reporting.parent.node_id.clone(),
        layer: reporting.parent.layer,
        parent: Some(reporting.parent.clone()),
        status,
        source_sequence,
    }))
}

/// Root-side fold of a parent terminal frame.
///
/// The exact `LayeredTerminalEvent` is preferred. A frame that carries the
/// parent identity instead of the child's execution identity is resolved
/// against the durable operation, and the redundant fields are cross-checked so
/// a mismatched frame can never fold the wrong node.
pub fn notice(snapshot: &LayeredSnapshot, input: &Value, id: &str) -> Result<LayeredTerminalEvent> {
    if let Ok(notice) = serde_json::from_value::<LayeredTerminalEvent>(input.clone()) {
        return Ok(notice);
    }
    let operation_id = input["operation_id"]
        .as_str()
        .context("operation_id required")?;
    let operation = snapshot
        .operations
        .iter()
        .find(|operation| operation.operation_id == operation_id)
        .context("unknown operation")?;
    ensure!(
        input["run_id"].as_str().is_none_or(|run| run == id),
        "terminal parent mismatch"
    );
    ensure!(
        input["node_id"]
            .as_str()
            .is_none_or(|node| node == operation.node_id),
        "terminal node mismatch"
    );
    ensure!(
        input["layer"]
            .as_u64()
            .is_none_or(|layer| layer == u64::from(operation.layer)),
        "terminal layer mismatch"
    );
    Ok(LayeredTerminalEvent {
        run_id: id.into(),
        operation_id: operation_id.into(),
        execution_kind: operation.execution_kind,
        execution_id: operation.execution_id.clone(),
        status: serde_json::from_value(input["status"].clone())
            .context("terminal status required")?,
        source_sequence: input["source_sequence"].as_u64().unwrap_or(0),
    })
}
