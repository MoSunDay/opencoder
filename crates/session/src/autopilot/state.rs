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
    /// One-shot review pass (`autopilot.mode = "review"`): not part of the
    /// PLAN → ACT → VERIFY loop.
    Review,
}

/// The verdict produced by the shadow VERIFY one-shot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyVerdict {
    /// The model said "no" — more work is needed, keep looping.
    MoreWork,
    /// The model said "yes" — the goal is fully achieved.
    Complete,
}

/// Why [`crate::autopilot::verify`] exhausted its retry budget. The two
/// causes need different remedies — `Unreachable` points at transport/auth
/// (network, rate limits, bad key), `Unparseable` at the judge model itself
/// answering with more than the single requested token. Both surface as
/// `ApOutcome::Aborted` with a cause-specific reason (no new event/outcome
/// variants) so logs and CLI output can tell them apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyFailure {
    /// Every attempt answered, but no answer parsed to a verdict.
    Unparseable { attempts: u32 },
    /// Every attempt failed before an answer arrived (transport/API error).
    Unreachable {
        attempts: u32,
        /// The last transport error, verbatim — the freshest diagnosis.
        last_error: String,
    },
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
