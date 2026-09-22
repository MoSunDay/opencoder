#![allow(dead_code)]
//! v4 fixtures: one two-layer canvas, the frozen descriptors control would
//! register for it, and the contexts control assembles for one decision.
use opencoder_core::{
    brain::{layered::*, BrainCapabilityDescriptor},
    fleet::ExecutionKind,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const SCAN: &str = "scan";
pub const REVIEW: &str = "review";

/// `scan -> review`: two layers, one node per layer, both bound to an agent.
pub fn request(_id: &str) -> Value {
    json!({
        "schema_version": 4,
        "plan": {
            "schema_version": 4,
            "title": "layered review",
            "objective": "inspect repository",
            "inputs": {"repo": "opencoder"},
            "nodes": [
                {
                    "node_id": SCAN,
                    "title": "scan the repository",
                    "capability_id": "cap-scan",

                    "retry": {"max_attempts": 2}
                },
                {
                    "node_id": REVIEW,
                    "title": "review the scan",
                    "capability_id": "cap-review",
                    }
            ],
            "edges": [{"from": SCAN, "to": REVIEW}],
            "max_rounds": 4
        },
        "inputs": {"repo": "opencoder"}
    })
}

pub fn parse(id: &str) -> LayeredRequest {
    serde_json::from_value(request(id)).expect("v4 fixture request")
}

/// One frozen descriptor per plan node, in plan order.
pub fn catalog() -> Vec<BrainCapabilityDescriptor> {
    vec![
        capability("cap-scan", "repo"),
        capability("cap-review", "source"),
    ]
}

pub fn capability(capability_id: &str, required: &str) -> BrainCapabilityDescriptor {
    BrainCapabilityDescriptor {
        capability_id: capability_id.into(),
        kind: ExecutionKind::Agent,
        target: "act".into(),
        input_desc: "bounded task input".into(),
        output_desc: "bounded task output".into(),
        required_inputs: vec![required.into()],
        definition: Value::Null,
        version: "1".into(),
    }
}

pub fn total_layers(id: &str) -> u32 {
    opencoder_brain::layered::layers(&parse(id).plan)
        .expect("plan layers")
        .len() as u32
}

/// Control builds the next layer context through the same domain helper the
/// validators use, so a node can never be handed a context control could not
/// build. The closing context (every layer dispatched) has no nodes left.
pub fn next_context(id: &str, snapshot: &LayeredSnapshot) -> LayeredContext {
    if snapshot.run.layer >= total_layers(id) {
        return closing(id, snapshot);
    }
    opencoder_brain::layered::layer_context(snapshot, &parse(id), &catalog(), BTreeMap::new(), None)
        .expect("next layer context")
}

fn closing(id: &str, snapshot: &LayeredSnapshot) -> LayeredContext {
    LayeredContext {
        schema_version: LAYERED_SCHEMA_VERSION,
        run_id: id.into(),
        generation: snapshot.run.generation,
        layer: snapshot.run.layer + 1,
        total_layers: total_layers(id),
        request: parse(id),
        nodes: vec![],
        todo: None,
        summaries: BTreeMap::new(),
        operations: snapshot.operations.clone(),
    }
}

/// A dispatch frame is exactly the operation, its assignment and its frozen
/// capability; control needs nothing else to create the child.
pub fn operation(frame: &Value) -> LayeredOperation {
    serde_json::from_value(frame["operation"].clone()).expect("dispatch operation")
}

/// The terminal notice control folds back for a child that already settled.
pub fn terminal_notice(
    operation: &LayeredOperation,
    status: LayeredOperationStatus,
    source_sequence: u64,
) -> LayeredTerminalEvent {
    LayeredTerminalEvent {
        run_id: operation.run_id.clone(),
        operation_id: operation.operation_id.clone(),
        execution_kind: operation.execution_kind,
        execution_id: operation.execution_id.clone(),
        status,
        source_sequence,
    }
}
