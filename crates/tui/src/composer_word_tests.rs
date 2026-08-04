//! Unit tests for readline-style word movement (`forward_word`, `backward_word`).

use super::{backward_word, forward_word};

// ---- forward_word ----

#[test]
fn forward_word_basic() {
    // "foo bar baz", cursor 0 → end of "foo" (3).
    assert_eq!(forward_word("foo bar baz", 0), 3);
}

#[test]
fn forward_word_from_whitespace() {
    // "foo  bar", cursor 3 (on first space) → end of "bar" (8).
    assert_eq!(forward_word("foo  bar", 3), 8);
}

#[test]
fn forward_word_mid_word() {
    // "foo bar", cursor 1 (middle of "foo") → end of "foo" (3).
    assert_eq!(forward_word("foo bar", 1), 3);
}

#[test]
fn forward_word_at_end() {
    // "foo", cursor 3 → already at end (3).
    assert_eq!(forward_word("foo", 3), 3);
}

#[test]
fn forward_word_punct_separate() {
    // "foo.bar", cursor 0 → end of "foo" (3); '.' is a separate Punct run.
    assert_eq!(forward_word("foo.bar", 0), 3);
}

#[test]
fn forward_word_through_punct() {
    // "foo.bar", cursor 3 (on '.') → land past '.' at the start of "bar" (4).
    assert_eq!(forward_word("foo.bar", 3), 4);
}

#[test]
fn forward_word_empty() {
    assert_eq!(forward_word("", 0), 0);
}

#[test]
fn forward_word_all_whitespace() {
    // "   ", cursor 0 → skip to the end (3).
    assert_eq!(forward_word("   ", 0), 3);
}

#[test]
fn forward_word_multibyte() {
    // "你好 world", cursor 0 → end of "你好" (2); each CJK char counts as 1 index.
    assert_eq!(forward_word("你好 world", 0), 2);
}

#[test]
fn forward_word_multiple_words() {
    // "one two three": repeated forward_word hops 0 → 3 → 7 → 13.
    let input = "one two three";
    assert_eq!(forward_word(input, 0), 3);
    assert_eq!(forward_word(input, 3), 7);
    assert_eq!(forward_word(input, 7), 13);
}

// ---- backward_word ----

#[test]
fn backward_word_basic() {
    // "foo bar", cursor 7 → start of "bar" (4).
    assert_eq!(backward_word("foo bar", 7), 4);
}

#[test]
fn backward_word_from_mid_word() {
    // "foo bar", cursor 5 (middle of "bar") → start of "bar" (4).
    assert_eq!(backward_word("foo bar", 5), 4);
}

#[test]
fn backward_word_cross_whitespace() {
    // "foo  bar", cursor 5 (on 'b') → step back through the whitespace run
    // and land on the first character of "foo" (0).
    assert_eq!(backward_word("foo  bar", 5), 0);
}

#[test]
fn backward_word_at_start() {
    assert_eq!(backward_word("foo", 0), 0);
}

#[test]
fn backward_word_trailing_whitespace() {
    // "foo  ", cursor 5 → skip the trailing whitespace back to "foo" (0).
    assert_eq!(backward_word("foo  ", 5), 0);
}

#[test]
fn backward_word_punct() {
    // "foo.bar", cursor 7 → step back through "bar" to its start (4).
    assert_eq!(backward_word("foo.bar", 7), 4);
}

#[test]
fn backward_word_from_punct() {
    // "foo.bar", cursor 4 (on 'b') → step back onto '.' (3).
    assert_eq!(backward_word("foo.bar", 4), 3);
}

#[test]
fn backward_word_empty() {
    assert_eq!(backward_word("", 0), 0);
}

#[test]
fn backward_word_all_whitespace() {
    // "  foo", cursor 2 (on 'f') → step back into leading whitespace (0).
    assert_eq!(backward_word("  foo", 2), 0);
}

#[test]
fn backward_word_multibyte() {
    // "你好 world", cursor 7 → step back through "world" to its start (3).
    assert_eq!(backward_word("你好 world", 7), 3);
}

#[test]
fn backward_word_newline_boundary() {
    // "foo\nbar", cursor 7 → step back through "bar" to its start (4).
    assert_eq!(backward_word("foo\nbar", 7), 4);
}
