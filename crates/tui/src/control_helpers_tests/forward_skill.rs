//! Tests for `forward_skill_if_compound`.
use super::forward_skill_if_compound;

#[test]
fn compound_sandbox_with_skill_forwards_raw() {
    let got = forward_skill_if_compound("/sandbox $review", "/sandbox");
    assert_eq!(got, "/sandbox $review");
}

#[test]
fn compound_sandbox_with_text_forwards_raw() {
    let got = forward_skill_if_compound("/sandbox $review do stuff", "/sandbox do stuff");
    assert_eq!(got, "/sandbox $review do stuff");
}

#[test]
fn compound_act_with_skill_forwards_raw() {
    let got = forward_skill_if_compound("/act $review", "/act");
    assert_eq!(got, "/act $review");
}

#[test]
fn bare_command_not_forwarded() {
    let got = forward_skill_if_compound("/sandbox", "/sandbox");
    assert_eq!(got, "/sandbox");
}

#[test]
fn plain_text_untouched() {
    let got = forward_skill_if_compound("hello world", "hello world");
    assert_eq!(got, "hello world");
}
