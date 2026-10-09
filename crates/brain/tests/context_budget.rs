use opencoder_brain::layered::{self, budget::*};
use opencoder_core::{
    brain::{layered::*, BrainCapabilityDescriptor},
    fleet::ExecutionKind,
};
use serde_json::json;

fn request(layers: usize, width: usize) -> LayeredRequest {
    serde_json::from_value(json!({"schema_version":7,"plan":{"schema_version":7,"title":"capacity","objective":"retain all evidence","max_rounds":5,
        "layers":(0..layers).map(|layer| json!({"layer_id":format!("l{layer}"),"title":"milestone","task":"perform work","objective":"verify","success_criteria":"all outputs verified"})).collect::<Vec<_>>(),
        "nodes":(0..layers).flat_map(|layer| (0..width).map(move |node| json!({"node_id":format!("n{layer}-{node}"),"layer_id":format!("l{layer}"),"title":"work","objective":"work","capability_id":"agent"}))).collect::<Vec<_>>(),"edges":[]}})).unwrap()
}

fn catalog(kind: ExecutionKind) -> Vec<BrainCapabilityDescriptor> {
    vec![serde_json::from_value(json!({"capability_id":"agent","kind":kind,"target":"act","summary":"bounded output","input_desc":"task","output_desc":"evidence","definition":{},"version":"1"})).unwrap()]
}

#[test]
fn valid_wide_plan_is_rejected_before_any_child_can_run() {
    let request = request(3, 32);
    layered::validate_request(&request).unwrap();
    let output = json!({"payload":"x".repeat(12_000)});
    opencoder_brain::contracts::validate_output(&[], &output).unwrap();
    let error = admission_budget(&request, &catalog(ExecutionKind::Agent), 4_000_000).unwrap_err();
    assert!(error.to_string().contains("complete-evidence capacity"));
}

#[test]
fn model_capacity_and_json_escaping_are_included_in_admission() {
    let request = request(1, 1);
    let caps = catalog(ExecutionKind::Agent);
    let budget = admission_budget(&request, &caps, 128_000).unwrap();
    assert!(admission_budget(&request, &caps, 32_000).is_err());
    let initial = layered::initialize("brain-budget-test", &request, 1).unwrap();
    let snapshot = LayeredSnapshot {
        schema_version: 7,
        run: initial.run,
        operations: vec![],
    };
    let mut context =
        layered::layer_context(&snapshot, &request, &caps, Default::default(), None).unwrap();
    let evidence = json!({"payload":"\0".repeat(2700)});
    opencoder_brain::contracts::validate_output(&[], &evidence).unwrap();
    context
        .summaries
        .insert("agent-evidence".into(), evidence.to_string());
    assert!(layered::instruction(&context).unwrap().len() < budget.instruction_bytes);
    validate_context(&context, budget.context_limit).unwrap();
    budget.check_guidance(budget.guidance_bytes).unwrap();
    assert!(budget.check_guidance(budget.guidance_bytes + 1).is_err());
    assert!(human_input_bytes("\0").unwrap() > human_input_bytes("a").unwrap());
}

#[test]
fn nested_receipts_and_large_frozen_definitions_are_reserved() {
    let request = request(1, 1);
    let leaf = admission_budget(&request, &catalog(ExecutionKind::Agent), 1_000_000).unwrap();
    let nested = admission_budget(&request, &catalog(ExecutionKind::Brain), 1_000_000).unwrap();
    assert!(nested.instruction_bytes > leaf.instruction_bytes + 100_000);
    let mut caps = catalog(ExecutionKind::Agent);
    caps[0].definition = json!({"source":"x".repeat(FRAME_BYTES)});
    assert!(admission_budget(&request, &caps, 4_000_000).is_err());
}
