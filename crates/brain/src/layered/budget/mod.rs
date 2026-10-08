//! Conservative admission for complete evidence in one model activation.
use anyhow::{ensure, Result};
use opencoder_core::{
    brain::{layered::*, BrainCapabilityDescriptor, BRAIN_EVIDENCE_MAX_BYTES},
    fleet::ExecutionKind,
};
use serde::{Deserialize, Serialize};

pub const CAPABILITY: &str = "brain_context_budget_v1";
pub const INSTRUCTION_BYTES: usize = 1024 * 1024;
pub const FRAME_BYTES: usize = 2 * 1024 * 1024;
pub const OUTPUT_TOKENS: u64 = 16_384;
// Run reflection, validation feedback, IDs and message framing grow after admission.
const RUN_RESERVE: usize = 56 * 1024;
const OPERATION_RESERVE: usize = 2048;
// A nested receipt includes a summary of up to 16384 Unicode characters.
const NESTED_EVIDENCE: usize = 2 * (6 * 16_384 + 2048);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextBudget {
    pub context_limit: u64,
    pub instruction_bytes: usize,
    pub frame_bytes: usize,
    pub guidance_bytes: usize,
}

impl ContextBudget {
    pub fn check_guidance(&self, bytes: usize) -> Result<()> {
        ensure!(bytes <= self.guidance_bytes,
            "Brain context capacity exhausted: guidance requires {bytes} bytes, remaining reservation is {}. Use a smaller plan or a model with a larger context window; evidence is retained.", self.guidance_bytes);
        Ok(())
    }
}

pub fn admission_budget(
    request: &LayeredRequest,
    capabilities: &[BrainCapabilityDescriptor],
    context_limit: u64,
) -> Result<ContextBudget> {
    super::validate_request(request)?;
    let initial = super::initialize("brain-budget", request, 0)?;
    let snapshot = LayeredSnapshot {
        schema_version: LAYERED_SCHEMA_VERSION,
        run: initial.run,
        operations: vec![],
    };
    let context = super::layer_context(&snapshot, request, capabilities, Default::default(), None)?;
    let mut evidence = 0;
    for node in &request.plan.nodes {
        let capability = capabilities
            .iter()
            .find(|cap| cap.capability_id == node.capability_id)
            .ok_or_else(|| anyhow::anyhow!("frozen capability {} missing", node.capability_id))?;
        // JSON evidence becomes a JSON string in the context. Escaping may double
        // its serialized size. Strings already satisfy the same JSON byte limit.
        evidence += OPERATION_RESERVE
            + if capability.kind == ExecutionKind::Brain {
                NESTED_EVIDENCE
            } else {
                2 * BRAIN_EVIDENCE_MAX_BYTES
            };
    }
    // TODO draft is 4096 characters; title is bounded at the context producer.
    let todo_reserve = if request.plan.todo.is_some() {
        6 * (4096 + 1024) + 1024
    } else {
        0
    };
    let growth = evidence + RUN_RESERVE + todo_reserve;
    let instruction_bytes = super::instruction(&context)?.len() + growth;
    let frame_bytes = serde_json::to_vec(&context)?.len() + growth + 4096;
    let tokens = instruction_bytes as u64 + super::PROMPT.len() as u64 + OUTPUT_TOKENS;
    ensure!(instruction_bytes <= INSTRUCTION_BYTES && frame_bytes <= FRAME_BYTES && tokens <= context_limit,
        "Brain plan exceeds complete-evidence capacity: instruction {instruction_bytes}/{INSTRUCTION_BYTES} bytes, frame {frame_bytes}/{FRAME_BYTES} bytes, conservative tokens {tokens}/{context_limit} (including {OUTPUT_TOKENS} output tokens). Reduce plan nodes/inputs/contracts or select a model with a larger context window before starting.");
    let guidance_bytes = (INSTRUCTION_BYTES - instruction_bytes)
        .min(FRAME_BYTES - frame_bytes)
        .min((context_limit - tokens) as usize);
    Ok(ContextBudget {
        context_limit,
        instruction_bytes,
        frame_bytes,
        guidance_bytes,
    })
}

/// Reserve the future guidance note as well as the accepted human input.
pub fn human_input_bytes(text: &str) -> Result<usize> {
    Ok(serde_json::to_vec(text)?.len() + 6 * 1024 + 32)
}

pub fn validate_context(context: &LayeredContext, context_limit: u64) -> Result<()> {
    let prompt = super::instruction(context)?;
    let tokens = (prompt.len() + super::PROMPT.len() + 1024) as u64 + OUTPUT_TOKENS;
    ensure!(tokens <= context_limit, "Brain complete context needs at most {tokens} tokens, model limit is {context_limit}; evidence retained, use a smaller plan or larger context window");
    ensure!(
        serde_json::to_vec(context)?.len() + 4096 <= FRAME_BYTES,
        "Brain complete context exceeds transport capacity; evidence retained"
    );
    Ok(())
}
