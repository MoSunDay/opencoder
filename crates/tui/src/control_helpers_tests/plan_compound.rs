//! Tests for `plan_compound_for_submit`: only a compound `/plan <content>`
//! submission yields a prompt to submit; bare or non-plan inputs fall back to
//! a normal mode toggle (`None`).
use super::*;

#[test]
fn plan_with_skill_token_is_compound() {
    assert_eq!(
        plan_compound_for_submit("/plan $review"),
        Some("/plan $review".to_string())
    );
}

#[test]
fn plan_with_plain_text_is_compound() {
    assert_eq!(
        plan_compound_for_submit("/plan fix the bug"),
        Some("/plan fix the bug".to_string())
    );
}

#[test]
fn plan_with_skill_and_text_is_compound() {
    assert_eq!(
        plan_compound_for_submit("/plan $review do stuff"),
        Some("/plan $review do stuff".to_string())
    );
}

#[test]
fn bare_plan_is_not_compound() {
    assert_eq!(plan_compound_for_submit("/plan"), None);
}

#[test]
fn whitespace_padded_bare_plan_is_not_compound() {
    assert_eq!(plan_compound_for_submit("/plan   "), None);
}

#[test]
fn padded_bare_plan_is_not_compound() {
    assert_eq!(plan_compound_for_submit("  /plan  "), None);
}

#[test]
fn act_compound_is_not_plan() {
    // `/act <content>` is intentionally not handled here: it would bypass the
    // plan->act handoff logic.
    assert_eq!(plan_compound_for_submit("/act fix the bug"), None);
}

#[test]
fn bare_act_clear_context_is_not_plan() {
    assert_eq!(plan_compound_for_submit("/act_clear_context"), None);
}

#[test]
fn plain_text_is_not_compound() {
    assert_eq!(plan_compound_for_submit("fix the bug"), None);
}

#[test]
fn empty_string_is_not_compound() {
    assert_eq!(plan_compound_for_submit(""), None);
}

#[test]
fn padded_plan_with_content_is_trimmed_and_compound() {
    assert_eq!(
        plan_compound_for_submit("  /plan review  "),
        Some("/plan review".to_string())
    );
}
