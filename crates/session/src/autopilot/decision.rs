//! Pure decision functions: verdict parsing and loop-control. These are the
//! most logic-heavy part of autopilot and must stay free of I/O so they are
//! trivially unit-testable.

use crate::autopilot::state::{ApOutcome, VerifyFailure, VerifyVerdict};

/// Parse the VERIFY model's raw answer into a boolean verdict.
///
/// The bool is the model's *affirmative* answer to the question asked. VERIFY
/// asks "is the goal fully achieved?", so `true` = the goal IS achieved
/// (task complete) and `false` = more work is still needed. The prompt asks
/// for a single "yes"/"no" token; parsing is STRICT — after trimming
/// surrounding whitespace and trailing sentence punctuation, the ENTIRE
/// answer must be exactly one accepted token (case-insensitive). Qualified
/// answers ("Yes, but the tests fail", "no — keep going") return `None`
/// (malformed → retry): the old first-token heuristic graded "Yes, but…" as
/// Complete.
pub fn parse_verdict(text: &str) -> Option<bool> {
    let raw = text.trim();
    if raw.is_empty() {
        return None;
    }
    // With `max_tokens = 8` the answer is tiny, so one trailing punctuation
    // run ("yes." / "no!") is tolerated, anything longer is not.
    let bare = raw.trim_end_matches(['.', ',', '!', '?', ';', ':']).trim();
    match bare.to_lowercase().as_str() {
        "yes" | "y" | "true" | "1" | "是" => Some(true),
        "no" | "n" | "false" | "0" | "否" => Some(false),
        _ => None,
    }
}

/// Decide whether the loop should stop given a VERIFY result and the current
/// iteration count.
///
/// - `Ok(Complete)` → stop with [`ApOutcome::Complete`]
/// - `Err(Unparseable | Unreachable)` → stop with [`ApOutcome::Aborted`] whose
///   reason names the exhausted cause (retries already spent)
/// - `Ok(MoreWork)` → stop with [`ApOutcome::MaxIterations`] if the next
///   iteration would meet/exceed `max`; otherwise `None` (keep looping)
pub fn should_stop(
    verdict: Result<VerifyVerdict, VerifyFailure>,
    iteration: u32,
    max: u32,
) -> Option<ApOutcome> {
    match verdict {
        Ok(VerifyVerdict::Complete) => Some(ApOutcome::Complete),
        Err(VerifyFailure::Unparseable { attempts }) => Some(ApOutcome::Aborted(format!(
            "verify verdict unparseable after {attempts} attempts"
        ))),
        Err(VerifyFailure::Unreachable {
            attempts,
            last_error,
        }) => Some(ApOutcome::Aborted(format!(
            "verify judge unreachable after {attempts} attempts (last error: {last_error})"
        ))),
        Ok(VerifyVerdict::MoreWork) => {
            if max == 0 || iteration + 1 >= max {
                Some(ApOutcome::MaxIterations)
            } else {
                None
            }
        }
    }
}
