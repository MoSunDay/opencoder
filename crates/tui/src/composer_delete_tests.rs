//! Tests for `delete_word_back`.

use super::super::*;

#[test]
fn delete_word_back_basic() {
    // "hello world|" → "hello |"
    let (s, i) = delete_word_back("hello world", 11).unwrap();
    assert_eq!(s, "hello ");
    assert_eq!(i, 6);
}

#[test]
fn delete_word_back_single_word() {
    // "hello|" → ""
    let (s, i) = delete_word_back("hello", 5).unwrap();
    assert_eq!(s, "");
    assert_eq!(i, 0);
}

#[test]
fn delete_word_back_trailing_whitespace() {
    // "hello   |" → "" (deletes word + trailing spaces, like bash)
    let (s, i) = delete_word_back("hello   ", 8).unwrap();
    assert_eq!(s, "");
    assert_eq!(i, 0);
}

#[test]
fn delete_word_back_mid_word() {
    // "hello wo|rld" → "hello |rld"
    let (s, i) = delete_word_back("hello world", 8).unwrap();
    assert_eq!(s, "hello rld");
    assert_eq!(i, 6);
}

#[test]
fn delete_word_back_after_space() {
    // "hello |world" → "|world" (deletes "hello " including the space)
    let (s, i) = delete_word_back("hello world", 6).unwrap();
    assert_eq!(s, "world");
    assert_eq!(i, 0);
}

#[test]
fn delete_word_back_at_line_start_returns_none() {
    // Cursor at start of first line → nothing to delete
    assert!(delete_word_back("hello", 0).is_none());
}

#[test]
fn delete_word_back_empty_input_returns_none() {
    assert!(delete_word_back("", 0).is_none());
}

#[test]
fn delete_word_back_does_not_cross_newline() {
    // "line1\nline2|" → "line1\n" (only deletes "line2")
    let (s, i) = delete_word_back("line1\nline2", 11).unwrap();
    assert_eq!(s, "line1\n");
    assert_eq!(i, 6);
}

#[test]
fn delete_word_back_at_second_line_start_returns_none() {
    // "line1\n|line2" → None (cursor at start of second line)
    assert!(delete_word_back("line1\nline2", 6).is_none());
}

#[test]
fn delete_word_back_multibyte_chars() {
    // "你好 world|" → "你好 |"
    let (s, i) = delete_word_back("你好 world", 8).unwrap();
    assert_eq!(s, "你好 ");
    assert_eq!(i, 3);
}

#[test]
fn delete_word_back_only_whitespace_before_cursor() {
    // "hello\n   |" → "hello\n" (deletes trailing spaces on current line)
    let (s, i) = delete_word_back("hello\n   ", 9).unwrap();
    assert_eq!(s, "hello\n");
    assert_eq!(i, 6);
}

#[test]
fn delete_word_back_consecutive_presses() {
    // Simulate pressing Ctrl+W twice on "hello world"
    let (s1, i1) = delete_word_back("hello world", 11).unwrap();
    assert_eq!(s1, "hello ");
    assert_eq!(i1, 6);
    // Second press: "hello |" → "|"
    let (s2, i2) = delete_word_back(&s1, i1).unwrap();
    assert_eq!(s2, "");
    assert_eq!(i2, 0);
}
