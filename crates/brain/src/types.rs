//! Wire-shape types for the brain crate. Pure data only — behaviour lives in
//! `domain` (pure functions) and `runtime` (I/O orchestration).

use serde::Deserialize;

/// Payload for creating or updating a brain capability. Field-level rules
/// (non-empty, length caps, count caps) live in [`crate::domain::validate`].
#[derive(Debug, Clone, Deserialize)]
pub struct CapabilityInput {
    /// Coarse capability category, e.g. "tool-usage" or "debugging".
    pub capability_type: String,
    /// One-sentence description of the capability — the primary search key.
    pub summary: String,
    /// What kind of input exercises this capability.
    pub input_desc: String,
    /// What a successful exercise of the capability produces.
    pub output_desc: String,
    /// Ordered exemplar inputs (few-shot / replay material).
    pub eng_inputs: Vec<String>,
}
