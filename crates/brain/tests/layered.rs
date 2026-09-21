use opencoder_brain::layered;
use opencoder_core::brain::layered::*;
use opencoder_core::brain::BrainCapabilityDescriptor;
use opencoder_core::fleet::ExecutionKind;
use serde_json::json;
use std::collections::BTreeMap;

#[path = "layered/validate.rs"]
mod validate;

#[path = "layered/barriers.rs"]
mod barriers;

#[path = "layered/terminal.rs"]
mod terminal;

fn node(id: &str, capability: &str) -> LayeredNode {
    LayeredNode {
        node_id: id.into(),
        title: id.into(),
        capability_id: capability.into(),
        instructions: String::new(),
        retry: LayeredRetry::default(),
    }
}

fn edge(from: &str, to: &str) -> LayeredEdge {
    LayeredEdge {
        from: from.into(),
        to: to.into(),
    }
}

/// impact -> fix -> verify, plus a parallel pair on the first layer.
fn plan() -> LayeredPlan {
    LayeredPlan {
        schema_version: 4,
        title: "repair".into(),
        objective: "repair and verify".into(),
        inputs: [("repo".into(), json!("example"))].into(),
        todo: Some(LayeredTodoRef { id: "pt-1".into() }),
        nodes: vec![
            node("impact", "agent-impact"),
            node("audit", "agent-audit"),
            node("fix", "plan-fix"),
            node("verify", "dag-regression"),
        ],
        edges: vec![
            edge("impact", "fix"),
            edge("audit", "fix"),
            edge("fix", "verify"),
        ],
        max_rounds: 8,
    }
}

/// Two independent branches: `impact -> fix` and `audit -> check`.
fn branching_plan() -> LayeredPlan {
    LayeredPlan {
        schema_version: 4,
        title: "branches".into(),
        objective: "two independent branches".into(),
        inputs: [("repo".into(), json!("example"))].into(),
        todo: None,
        nodes: vec![
            node("impact", "agent-impact"),
            node("audit", "agent-audit"),
            node("fix", "plan-fix"),
            node("check", "dag-regression"),
        ],
        edges: vec![edge("impact", "fix"), edge("audit", "check")],
        max_rounds: 8,
    }
}

fn branching_request() -> LayeredRequest {
    LayeredRequest {
        schema_version: 4,
        plan: branching_plan(),
        inputs: BTreeMap::new(),
        artifacts: BTreeMap::new(),
        origin: None,
        parent: None,
        depth: 0,
    }
}

fn request() -> LayeredRequest {
    LayeredRequest {
        schema_version: 4,
        plan: plan(),
        inputs: BTreeMap::new(),
        artifacts: BTreeMap::new(),
        origin: None,
        parent: None,
        depth: 0,
    }
}

fn cap(id: &str, kind: ExecutionKind, required: &[&str]) -> BrainCapabilityDescriptor {
    BrainCapabilityDescriptor {
        capability_id: id.into(),
        kind,
        target: "act".into(),
        input_desc: "repo".into(),
        output_desc: "result".into(),
        required_inputs: required.iter().map(|name| name.to_string()).collect(),
        definition: json!({"name": id}),
        version: "v1".into(),
    }
}

fn catalog() -> Vec<BrainCapabilityDescriptor> {
    vec![
        cap("agent-impact", ExecutionKind::Agent, &[]),
        cap("agent-audit", ExecutionKind::Agent, &[]),
        cap("plan-fix", ExecutionKind::Agent, &[]),
        cap("dag-regression", ExecutionKind::Dag, &[]),
    ]
}

fn initialized() -> LayeredSnapshot {
    let change = layered::initialize("brain-layered", &request(), 1).unwrap();
    LayeredSnapshot {
        schema_version: 4,
        run: change.run,
        operations: change.operations,
    }
}

fn deciding() -> LayeredSnapshot {
    let snapshot = initialized();
    let mut run = snapshot.run.clone();
    run.phase = LayeredPhase::Deciding;
    run.generation = 1;
    LayeredSnapshot { run, ..snapshot }
}

fn dispatch(layer: u32, nodes: &[&str]) -> LayeredDecision {
    LayeredDecision::DispatchLayer {
        layer,
        assignments: nodes
            .iter()
            .map(|node_id| LayeredAssignment {
                node_id: node_id.to_string(),
                inputs: BTreeMap::new(),
                reason: "layer dispatch".into(),
            })
            .collect(),
        reason: "dispatch".into(),
        evidence_execution_ids: vec![],
    }
}

fn assignment(node_id: &str, execution_id: &str) -> LayeredAssignment {
    LayeredAssignment {
        node_id: node_id.into(),
        inputs: [(
            "input".to_string(),
            opencoder_core::brain::BrainInputBinding::Execution {
                execution_id: execution_id.into(),
                path: String::new(),
            },
        )]
        .into(),
        reason: "bind upstream".into(),
    }
}

fn apply(_snapshot: &LayeredSnapshot, change: LayeredChange) -> LayeredSnapshot {
    LayeredSnapshot {
        schema_version: 4,
        run: change.run,
        operations: change.operations,
    }
}

fn notice(
    op: &LayeredOperation,
    status: LayeredOperationStatus,
    sequence: u64,
) -> LayeredTerminalEvent {
    LayeredTerminalEvent {
        run_id: op.run_id.clone(),
        operation_id: op.operation_id.clone(),
        execution_kind: op.execution_kind,
        execution_id: op.execution_id.clone(),
        status,
        source_sequence: sequence,
    }
}
