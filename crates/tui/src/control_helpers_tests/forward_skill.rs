//! Tests for `forward_skill_if_compound`.
use super::forward_skill_if_compound;

#[test]
fn compound_plan_with_skill_forwards_raw() {
    let got = forward_skill_if_compound("/plan $review", "/plan");
    assert_eq!(got, "/plan $review");
}

#[test]
fn compound_plan_with_text_forwards_raw() {
    let got = forward_skill_if_compound("/plan $review do stuff", "/plan do stuff");
    assert_eq!(got, "/plan $review do stuff");
}

#[test]
fn compound_act_with_skill_forwards_raw() {
    let got = forward_skill_if_compound("/act $review", "/act");
    assert_eq!(got, "/act $review");
}

#[test]
fn bare_command_not_forwarded() {
    let got = forward_skill_if_compound("/plan", "/plan");
    assert_eq!(got, "/plan");
}

#[test]
fn plain_text_untouched() {
    let got = forward_skill_if_compound("hello world", "hello world");
    assert_eq!(got, "hello world");
}
