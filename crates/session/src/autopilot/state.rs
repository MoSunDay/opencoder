//! Pure data types for the autopilot loop. No I/O, no side effects — fully
//! unit-testable in isolation (see `decision.rs` + `tests.rs`).

use serde::{Deserialize, Serialize};

/// A phase within one PLAN -> ACT -> VERIFY iteration. Serialized into the
/// `SessionEvent::AutoPilot` payload so surfaces (TUI / web SSE) can render
/// progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApPhase {
    Plan,
    Act,
    Verify,
}

/// The verdict produced by the shadow VERIFY one-shot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyVerdict {
    /// The model said "yes" — more work is needed, keep looping.
    MoreWork,
    /// The model said "no" — the task is complete.
    Complete,
    /// Every retry produced an unparseable answer.
    Malformed,
}

/// Terminal outcome of [`crate::autopilot::drive`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApOutcome {
    /// VERIFY returned "no" — the goal is satisfied.
    Complete,
    /// VERIFY never produced a parseable verdict (`verify_retries` exhausted),
    /// or a phase run errored.
    Aborted(String),
    /// `max_iterations` reached without VERIFY saying "no".
    MaxIterations,
}

/// Mutable loop state, threaded through each iteration. `goal` is extracted
/// once from the initial user message and reused for the VERIFY prompt so the
/// judgement is anchored to the original intent rather than the latest turn.
#[derive(Debug, Clone)]
pub struct ApState {
    pub iteration: u32,
    pub goal: String,
}

impl ApState {
    pub fn new(goal: String) -> Self {
        ApState { iteration: 0, goal }
    }
}
