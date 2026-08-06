//! Tests for `is_pure_control_cmd`: only bare control commands suppress the
//! transcript echo; compound inputs that carry user content must be echoed.
use super::*;

#[test]
fn bare_plan_is_pure() {
    assert!(is_pure_control_cmd("/plan"));
}

#[test]
fn bare_act_is_pure() {
    assert!(is_pure_control_cmd("/act"));
}

#[test]
fn bare_act_clear_context_is_pure() {
    assert!(is_pure_control_cmd("/act_clear_context"));
}

#[test]
fn plan_with_skill_is_not_pure() {
    assert!(!is_pure_control_cmd("/plan $review"));
}

#[test]
fn plan_with_skill_and_text_is_not_pure() {
    assert!(!is_pure_control_cmd("/plan $review do stuff"));
}

#[test]
fn plan_with_plain_text_is_not_pure() {
    assert!(!is_pure_control_cmd("/plan fix the bug"));
}

#[test]
fn act_with_skill_is_not_pure() {
    assert!(!is_pure_control_cmd("/act $review"));
}

#[test]
fn plain_prompt_is_not_pure() {
    assert!(!is_pure_control_cmd("hello world"));
}

#[test]
fn whitespace_padded_bare_plan_is_pure() {
    assert!(is_pure_control_cmd("  /plan  "));
}

#[test]
fn empty_string_is_not_pure() {
    assert!(!is_pure_control_cmd(""));
}
