//! Unit tests for the pure decision functions (no I/O, no async).

use super::decision::{parse_verdict, should_stop};
use super::state::{ApOutcome, VerifyFailure, VerifyVerdict};

// ── parse_verdict ─────────────────────────────────────────────────────────

#[test]
fn parse_yes_variants() {
    for s in ["yes", "YES", "Yes", "y", "Y", "true", "TRUE", "1", "是"] {
        assert_eq!(parse_verdict(s), Some(true), "failed on {s:?}");
    }
}

#[test]
fn parse_no_variants() {
    for s in ["no", "NO", "No", "n", "N", "false", "FALSE", "0", "否"] {
        assert_eq!(parse_verdict(s), Some(false), "failed on {s:?}");
    }
}

#[test]
fn parse_tolerates_punctuation_and_whitespace() {
    assert_eq!(parse_verdict(" yes. "), Some(true));
    assert_eq!(parse_verdict("no!\n"), Some(false));
    assert_eq!(parse_verdict("yes..."), Some(true));
    assert_eq!(parse_verdict("\tno\t"), Some(false));
    assert_eq!(parse_verdict("  NO?  "), Some(false));
}

#[test]
fn parse_qualified_answers_are_malformed() {
    // Strict single-token: a qualifier after yes/no means the verdict is NOT
    // known. "Yes, more work" was graded Complete by the old first-token
    // heuristic — the exact false-complete this strictness removes.
    for s in [
        "Yes, more work",
        "yes but the tests fail",
        "No, keep going",
        "yes: complete",
        "yes no",
        "Yes - the goal is achieved",
    ] {
        assert_eq!(parse_verdict(s), None, "{s:?} should be None");
    }
}

#[test]
fn parse_garbage_and_empty_is_none() {
    for s in ["", "   ", "maybe", "kind of", "i think so", "yep", "done"] {
        assert_eq!(parse_verdict(s), None, "{s:?} should be None");
    }
}

// ── should_stop ───────────────────────────────────────────────────────────

#[test]
fn complete_stops() {
    assert_eq!(
        should_stop(Ok(VerifyVerdict::Complete), 0, 10),
        Some(ApOutcome::Complete)
    );
    assert_eq!(
        should_stop(Ok(VerifyVerdict::Complete), 5, 10),
        Some(ApOutcome::Complete)
    );
}

#[test]
fn unparseable_exhaustion_aborts_with_cause() {
    let out = should_stop(Err(VerifyFailure::Unparseable { attempts: 3 }), 0, 10);
    match out {
        Some(ApOutcome::Aborted(reason)) => {
            assert!(reason.contains("unparseable"), "reason: {reason}");
            assert!(reason.contains('3'), "reason names attempts: {reason}");
        }
        other => panic!("expected Aborted, got {other:?}"),
    }
}

#[test]
fn unreachable_exhaustion_aborts_with_cause() {
    let out = should_stop(
        Err(VerifyFailure::Unreachable {
            attempts: 2,
            last_error: "429 rate limited".into(),
        }),
        0,
        10,
    );
    match out {
        Some(ApOutcome::Aborted(reason)) => {
            assert!(reason.contains("unreachable"), "reason: {reason}");
            assert!(
                reason.contains("429 rate limited"),
                "reason carries the last error: {reason}"
            );
        }
        other => panic!("expected Aborted, got {other:?}"),
    }
}

#[test]
fn more_work_under_cap_continues() {
    assert_eq!(should_stop(Ok(VerifyVerdict::MoreWork), 0, 10), None);
    assert_eq!(should_stop(Ok(VerifyVerdict::MoreWork), 8, 10), None);
}

#[test]
fn more_work_at_cap_is_max_iterations() {
    // iteration 9, max 10 -> next (10) meets cap -> MaxIterations
    assert_eq!(
        should_stop(Ok(VerifyVerdict::MoreWork), 9, 10),
        Some(ApOutcome::MaxIterations)
    );
}

#[test]
fn more_work_with_zero_max_is_max_iterations() {
    assert_eq!(
        should_stop(Ok(VerifyVerdict::MoreWork), 0, 0),
        Some(ApOutcome::MaxIterations)
    );
}
