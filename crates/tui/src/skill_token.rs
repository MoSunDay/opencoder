//! Pure parser for inline `{$name}` skill tokens.
//!
//! Tokens may appear anywhere in the composer text. On submit they are
//! stripped and the referenced skills are loaded (see `app_helpers`).
//! This module is dependency-free and fully unit-tested so the stripping
//! contract stays pinned regardless of where tokens sit in the text.

/// Strip every `{$name}` token from `text`, returning the cleaned text and the
/// list of skill names in the order they appeared (empty names from `{$}` are
/// skipped; duplicates are preserved here and deduped by the caller).
///
/// An unclosed `{$abc` (no matching `}`) is treated as literal text — the
/// `{` is emitted verbatim and scanning continues. The scan is UTF-8 safe:
/// `{$` are ASCII so byte-level detection never splits a multi-byte char, and
/// non-token bytes are copied one char at a time along char boundaries.
pub fn extract_skill_tokens(text: &str) -> (String, Vec<String>) {
    let mut clean = String::with_capacity(text.len());
    let mut names = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        // `{$` are both ASCII; a match therefore always lands on a char
        // boundary, so the byte slice that follows is valid.
        if bytes[i] == b'{' && i + 1 < text.len() && bytes[i + 1] == b'$' {
            let after = i + 2;
            if let Some(rel) = text[after..].find('}') {
                let close = after + rel;
                let name = text[after..close].trim();
                if !name.is_empty() {
                    names.push(name.to_string());
                }
                i = close + 1;
                continue;
            }
            // No closing `}` — fall through and emit `{` as a literal.
        }
        let ch = text[i..].chars().next().unwrap();
        clean.push(ch);
        i += ch.len_utf8();
    }
    (clean, names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_tokens_returns_text_unchanged() {
        let (clean, names) = extract_skill_tokens("hello world");
        assert_eq!(clean, "hello world");
        assert!(names.is_empty());
    }

    #[test]
    fn lone_dollar_is_literal() {
        // A bare `$` without a leading `{` is not a token.
        let (clean, names) = extract_skill_tokens("price is $5");
        assert_eq!(clean, "price is $5");
        assert!(names.is_empty());
    }

    #[test]
    fn basic_token_stripped() {
        let (clean, names) = extract_skill_tokens("{$code}");
        assert_eq!(clean, "");
        assert_eq!(names, vec!["code"]);
    }

    #[test]
    fn token_mid_text_preserves_surrounding_text() {
        let (clean, names) = extract_skill_tokens("hello {$code} world");
        assert_eq!(clean, "hello  world");
        assert_eq!(names, vec!["code"]);
    }

    #[test]
    fn multiple_tokens_in_order() {
        let (clean, names) = extract_skill_tokens("{$a} then {$b} then {$a}");
        assert_eq!(clean, " then  then ");
        assert_eq!(names, vec!["a", "b", "a"]);
    }

    #[test]
    fn adjacent_tokens() {
        let (clean, names) = extract_skill_tokens("x{$a}{$b}y");
        assert_eq!(clean, "xy");
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn name_with_spaces_is_trimmed() {
        let (clean, names) = extract_skill_tokens("{$  spaced  }");
        assert_eq!(clean, "");
        assert_eq!(names, vec!["spaced"]);
    }

    #[test]
    fn empty_token_is_skipped() {
        // `{$}` has an empty name — it is consumed (not emitted) but records
        // no skill, so the user can use it as a no-op marker if desired.
        let (clean, names) = extract_skill_tokens("a{$}b");
        assert_eq!(clean, "ab");
        assert!(names.is_empty());
    }

    #[test]
    fn unclosed_token_is_literal() {
        let (clean, names) = extract_skill_tokens("{$abc");
        assert_eq!(clean, "{$abc");
        assert!(names.is_empty());
    }

    #[test]
    fn unclosed_token_followed_by_text() {
        let (clean, names) = extract_skill_tokens("x {$abc y");
        assert_eq!(clean, "x {$abc y");
        assert!(names.is_empty());
    }

    #[test]
    fn double_brace_not_a_token() {
        // `{{` is not `{$` — both braces are literal.
        let (clean, names) = extract_skill_tokens("{{a}}");
        assert_eq!(clean, "{{a}}");
        assert!(names.is_empty());
    }

    #[test]
    fn utf8_text_preserved() {
        let (clean, names) = extract_skill_tokens("你好 {$code} 世界");
        assert_eq!(clean, "你好  世界");
        assert_eq!(names, vec!["code"]);
    }

    #[test]
    fn token_with_dash_in_name() {
        let (clean, names) = extract_skill_tokens("{$code-review}");
        assert_eq!(clean, "");
        assert_eq!(names, vec!["code-review"]);
    }

    #[test]
    fn empty_input() {
        let (clean, names) = extract_skill_tokens("");
        assert_eq!(clean, "");
        assert!(names.is_empty());
    }
}
