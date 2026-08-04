//! Re-export of [`extract_skill_tokens`] from `opencoder_core`.
//!
//! The function was moved to core so both the TUI and CLI headless path can
//! share the same token-stripping logic.

pub use opencoder_core::extract_skill_tokens;

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
    fn basic_token_stripped() {
        let (clean, names) = extract_skill_tokens("$ssh-pty do stuff");
        assert_eq!(clean, " do stuff");
        assert_eq!(names, vec!["ssh-pty"]);
    }

    #[test]
    fn multiple_tokens_in_order() {
        let (clean, names) = extract_skill_tokens("$a x $b y");
        assert_eq!(clean, " x  y");
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn dollar_not_followed_by_lowercase_is_literal() {
        // `$` followed by a digit (not a lowercase letter) is literal text,
        // not a skill token.
        let (clean, names) = extract_skill_tokens("$5");
        assert_eq!(clean, "$5");
        assert!(names.is_empty());
    }

    #[test]
    fn trailing_dollar_is_literal() {
        // A bare `$` at the end of the input is literal (no name follows).
        let (clean, names) = extract_skill_tokens("a $");
        assert_eq!(clean, "a $");
        assert!(names.is_empty());
    }

    #[test]
    fn token_with_dash_in_name() {
        let (clean, names) = extract_skill_tokens("$chrome-headless");
        assert_eq!(clean, "");
        assert_eq!(names, vec!["chrome-headless"]);
    }
}
