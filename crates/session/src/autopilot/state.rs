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
    /// The model said "no" — more work is needed, keep looping.
    MoreWork,
    /// The model said "yes" — the goal is fully achieved.
    Complete,
    /// Every retry produced an unparseable answer.
    Malformed,
}

/// Terminal outcome of [`crate::autopilot::drive`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApOutcome {
    /// VERIFY said "yes" — the goal is fully achieved.
    Complete,
    /// VERIFY never produced a parseable verdict (`verify_retries` exhausted).
    /// Phase-run errors are NOT folded here: `drive` propagates them via `?`.
    Aborted(String),
    /// `max_iterations` reached without VERIFY saying "yes".
    MaxIterations,
    /// The session's cancellation token was tripped mid-loop.
    Cancelled,
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
