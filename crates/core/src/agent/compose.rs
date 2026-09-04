//! Prompt composition for file-based custom agents.
//!
//! The current prompt version of an agent stores up to three markdown
//! sections (`soul.md` / `how.md` / `output.md`); [`compose_prompt`] joins
//! the present ones into one prompt string. Pure function: no filesystem,
//! no globals — callers read the files and pass their contents in.

/// Compose the agent prompt from the three optional prompt sections.
///
/// Fixed section order `# Soul` → `# How` → `# Output`; absent or
/// blank-after-trim sections are skipped entirely (no orphan header), and
/// the present sections are joined by a single blank line:
///
/// ```text
/// # Soul
/// <soul trimmed>
///
/// # How
/// <how trimmed>
///
/// # Output
/// <output trimmed>
/// ```
///
/// All sections absent/blank → the empty string.
pub fn compose_prompt(soul: Option<&str>, how: Option<&str>, output: Option<&str>) -> String {
    /// One section rendered as `# <Title>\n<body>`, or `None` when the body
    /// is absent/blank (blank sections carry no meaning — dropping them
    /// keeps the prompt free of header-only noise).
    fn section(title: &str, body: Option<&str>) -> Option<String> {
        let body = body?.trim();
        if body.is_empty() {
            return None;
        }
        Some(format!("# {title}\n{body}"))
    }

    [
        section("Soul", soul),
        section("How", how),
        section("Output", output),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_three_present_in_fixed_order() {
        let p = compose_prompt(Some(" soul A "), Some("how B"), Some("output C"));
        assert_eq!(p, "# Soul\nsoul A\n\n# How\nhow B\n\n# Output\noutput C");
    }

    #[test]
    fn each_section_alone_and_middle_gap() {
        assert_eq!(compose_prompt(Some("S"), None, None), "# Soul\nS");
        assert_eq!(compose_prompt(None, Some("H"), None), "# How\nH");
        assert_eq!(compose_prompt(None, None, Some("O")), "# Output\nO");
        // A missing middle section must not leave a stray header behind.
        assert_eq!(
            compose_prompt(Some("S"), None, Some("O")),
            "# Soul\nS\n\n# Output\nO"
        );
    }

    #[test]
    fn absent_and_blank_sections_yield_empty_string() {
        assert_eq!(compose_prompt(None, None, None), "");
        assert_eq!(compose_prompt(Some("  \n\t "), Some(""), None), "");
    }

    /// Stability: identical inputs must produce the identical string —
    /// the composed prompt is persisted and compared across turns.
    #[test]
    fn composition_is_stable() {
        let a = compose_prompt(Some("x"), Some("y"), Some("z"));
        let b = compose_prompt(Some("x"), Some("y"), Some("z"));
        assert_eq!(a, b);
        assert_eq!(a, "# Soul\nx\n\n# How\ny\n\n# Output\nz");
    }
}
