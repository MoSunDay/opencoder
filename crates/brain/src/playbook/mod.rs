//! Pure playbook domain for the brain's dual-track scheduling.
//!
//! A [`PlaybookSpec`] is an orchestration graph: named steps with
//! `depends_on` edges, each step targeting one executor (agent, team, dag,
//! todos workflow or brain capability) under a prompt template. Fixed
//! playbooks are authored by hand; dynamic ones are minted by the LLM
//! planner for a situation and cached by digest. Everything here is pure —
//! validation, prompt rendering, the topology helpers in [`topology`] and
//! the trigger matching in [`trigger`] have no I/O, so the whole scheduling
//! contract is unit-testable without a store or an LLM (the same split
//! `plan.rs` uses for decision trees).

pub mod spec;
pub mod topology;
pub mod trigger;

pub use spec::{
    render_prompt, validate, validate_draft, PlaybookInput, PlaybookOrigin, PlaybookSpec,
    PlaybookStep, PlaybookTarget, PlaybookTrigger, MAX_CHAIN_DEPTH, MAX_NAME_CHARS,
    MAX_PLAYBOOK_NAME_CHARS, MAX_PROMPT_CHARS, MAX_STEPS, MAX_WIDTH, SCHEMA_VERSION,
};
pub use topology::{chain_depth, collapse_blocked, max_width, ready_steps, topo_order};
pub use trigger::{cosine_similarity, fires, match_text, scan, threshold};
