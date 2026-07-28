//! Pure decision functions: verdict parsing and loop-control. These are the
//! most logic-heavy part of autopilot and must stay free of I/O so they are
//! trivially unit-testable.

use crate::autopilot::state::{ApOutcome, VerifyVerdict};

/// Parse the VERIFY model's raw answer into a boolean verdict.
///
/// Semantics: `true` = more work needed; `false` = task complete. The VERIFY
/// prompt asks for a single "yes"/"no" token, but we tolerate a handful of
/// aliases and surrounding punctuation/case so a slightly chatty model still
/// parses. Anything else returns `None` (malformed → retry).
pub fn parse_verdict(text: &str) -> Option<bool> {
    let raw = text.trim().to_lowercase();
    if raw.is_empty() {
        return None;
    }
    // First token, split on whitespace or common sentence punctuation. With
    // `max_tokens = 8` the answer is tiny, so the first token is decisive.
    let first = raw
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | ',' | '!' | '?' | ';' | ':'))
        .next()
        .unwrap_or("");
    match first {
        "yes" | "y" | "true" | "1" | "是" => Some(true),
        "no" | "n" | "false" | "0" | "否" => Some(false),
        _ => None,
    }
}

/// Decide whether the loop should stop given a verdict and the current
/// iteration count.
///
/// - `Complete` → stop with [`ApOutcome::Complete`]
/// - `Malformed` → stop with [`ApOutcome::Aborted`] (retries already exhausted)
/// - `MoreWork` → stop with [`ApOutcome::MaxIterations`] if the next iteration
///   would meet/exceed `max`; otherwise `None` (keep looping)
pub fn should_stop(verdict: VerifyVerdict, iteration: u32, max: u32) -> Option<ApOutcome> {
    match verdict {
        VerifyVerdict::Complete => Some(ApOutcome::Complete),
        VerifyVerdict::Malformed => Some(ApOutcome::Aborted(
            "verify verdict unparseable after retries".into(),
        )),
        VerifyVerdict::MoreWork => {
            if max == 0 || iteration + 1 >= max {
                Some(ApOutcome::MaxIterations)
            } else {
                None
            }
        }
    }
}
