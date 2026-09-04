//! `compaction` / `output_streamline` config blocks.
//!
//! Extracted from `config.rs` so the main module stays under the line gate.
//! Pure serde structs + default fns; no behavior lives here beyond
//! serialization defaults (mirrors `agent.rs`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    #[serde(default = "default_true")]
    pub auto: bool,
    #[serde(default = "default_threshold")]
    pub context_threshold: u64,
    #[serde(default = "default_tail_turns")]
    pub tail_turns: u32,
    #[serde(default = "default_reserved")]
    pub reserved: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffer: Option<u64>,
}
impl Default for CompactionConfig {
    fn default() -> Self {
        CompactionConfig {
            auto: true,
            context_threshold: 80_000,
            tail_turns: 2,
            reserved: 20_000,
            buffer: None,
        }
    }
}
/// Per-message assistant-output streamlining. Deterministic, meaning-preserving
/// normalization applied to completed assistant text *after* it has been
/// streamed to the UI (so live display fidelity is untouched) and *before* it
/// is persisted / re-sent as context — shaving **input** token overhead on
/// every later turn. Fenced code blocks are passed through verbatim; only
/// prose whitespace/structure is touched, so it is a no-op on already-clean
/// text. Configured via the `output_streamline` field of [`Config`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputStreamlineConfig {
    /// Master switch. On by default — every rule is a no-op on clean text.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Strip trailing whitespace from each prose line.
    #[serde(default = "default_true")]
    pub trim_trailing: bool,
    /// Collapse runs of 2+ blank prose lines into a single blank line.
    #[serde(default = "default_true")]
    pub collapse_blank_lines: bool,
    /// Trim leading/trailing blank lines from the whole message.
    #[serde(default = "default_true")]
    pub trim_outer: bool,
    /// Collapse interior space/tab runs in prose to a single space (leading
    /// indentation is preserved). Off by default: opt-in "aggressive" mode.
    #[serde(default)]
    pub collapse_inline_ws: bool,
}

impl Default for OutputStreamlineConfig {
    fn default() -> Self {
        OutputStreamlineConfig {
            enabled: true,
            trim_trailing: true,
            collapse_blank_lines: true,
            trim_outer: true,
            collapse_inline_ws: false,
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_threshold() -> u64 {
    80_000
}
fn default_tail_turns() -> u32 {
    2
}
fn default_reserved() -> u64 {
    20_000
}
