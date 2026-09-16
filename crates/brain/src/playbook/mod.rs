//! Historical Playbook decoding and structural validation only.
//! New writes, triggers and execution use the v2 graph entry points.

pub mod spec;
pub mod topology;

pub use spec::{
    validate, validate_draft, PlaybookInput, PlaybookOrigin, PlaybookRoute, PlaybookRouteKind,
    PlaybookSpec, PlaybookStep, PlaybookTarget, PlaybookTrigger, MAX_CHAIN_DEPTH, MAX_ID_CHARS,
    MAX_MATCH_TEXT_CHARS, MAX_NAME_CHARS, MAX_PLAYBOOK_NAME_CHARS, MAX_PROMPT_CHARS, MAX_STEPS,
    MAX_TARGET_REF_CHARS, MAX_WIDTH, SCHEMA_VERSION,
};
pub use topology::{chain_depth, max_width, topo_order};
