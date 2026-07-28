//! Unit tests for the pure decision functions (no I/O, no async).

use super::decision::{parse_verdict, should_stop};
use super::state::{ApOutcome, VerifyVerdict};

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
    assert_eq!(parse_verdict("Yes, more work"), Some(true));
    assert_eq!(parse_verdict("\tno\t"), Some(false));
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
        should_stop(VerifyVerdict::Complete, 0, 10),
        Some(ApOutcome::Complete)
    );
    assert_eq!(
        should_stop(VerifyVerdict::Complete, 5, 10),
        Some(ApOutcome::Complete)
    );
}

#[test]
fn malformed_aborts() {
    let out = should_stop(VerifyVerdict::Malformed, 0, 10);
    match out {
        Some(ApOutcome::Aborted(_)) => {}
        other => panic!("expected Aborted, got {other:?}"),
    }
}

#[test]
fn more_work_under_cap_continues() {
    assert_eq!(should_stop(VerifyVerdict::MoreWork, 0, 10), None);
    assert_eq!(should_stop(VerifyVerdict::MoreWork, 8, 10), None);
}

#[test]
fn more_work_at_cap_is_max_iterations() {
    // iteration 9, max 10 -> next (10) meets cap -> MaxIterations
    assert_eq!(
        should_stop(VerifyVerdict::MoreWork, 9, 10),
        Some(ApOutcome::MaxIterations)
    );
}

#[test]
fn more_work_with_zero_max_is_max_iterations() {
    assert_eq!(
        should_stop(VerifyVerdict::MoreWork, 0, 0),
        Some(ApOutcome::MaxIterations)
    );
}
