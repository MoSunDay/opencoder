//! Deterministic, meaning-preserving streamlining of completed assistant text.
//!
//! Applied *after* the assistant text has been streamed live to the UI (so the
//! user always sees the verbatim original) and *before* it is persisted and
//! re-sent as context on later turns. The only goal is to shave **input** token
//! overhead on subsequent turns without altering meaning:
//!
//! - trailing whitespace is stripped from prose lines,
//! - runs of blank prose lines collapse to a single blank line,
//! - the message is trimmed of leading/trailing blank lines,
//! - fenced code blocks (` ``` ` / `~~~`) are passed through **byte-for-byte**,
//!   so code formatting, indentation, and embedded blank lines are untouched.
//!
//! Every rule is a pure no-op on already-clean text; behaviour is driven
//! entirely by [`OutputStreamlineConfig`]. Pure functions, no internal state.

use opencoder_core::OutputStreamlineConfig;

/// Streamline a completed assistant message.
///
/// Returns a new, possibly-shorter `String`. Cheap on clean input: each rule
/// only rewrites text whose pattern actually matches. When the config is
/// disabled or the input is empty the original is returned unchanged.
pub fn streamline(text: &str, cfg: &OutputStreamlineConfig) -> String {
    if !cfg.enabled || text.is_empty() {
        return text.to_string();
    }
    let lines = collect_lines(text, cfg);
    let lines = if cfg.collapse_blank_lines {
        collapse_blank_runs(lines)
    } else {
        lines
    };
    let mut out: String = lines.into_iter().map(|l| l.text).collect();
    if cfg.trim_outer {
        out = trim_outer_blanks(&out);
    }
    out
}

/// A processed line tagged with whether it belongs to a fenced code block.
struct LineOut {
    /// The line text, including its trailing `'\n'` when the source had one.
    text: String,
    code: bool,
}

fn collect_lines(text: &str, cfg: &OutputStreamlineConfig) -> Vec<LineOut> {
    let mut lines: Vec<LineOut> = Vec::new();
    let mut in_code = false;
    for line in text.split_inclusive('\n') {
        if is_fence_line(line) {
            // A fence delimiter toggles the code state. Delimiter lines
            // themselves are emitted verbatim.
            in_code = !in_code;
            lines.push(LineOut {
                text: line.to_string(),
                code: true,
            });
            continue;
        }
        if in_code {
            lines.push(LineOut {
                text: line.to_string(),
                code: true,
            });
        } else {
            lines.push(LineOut {
                text: streamline_prose_line(line, cfg),
                code: false,
            });
        }
    }
    lines
}

/// A CommonMark fence delimiter: a line whose first non-whitespace run is three
/// or more backticks or tildos.
fn is_fence_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// Normalize a single prose line (which still carries its trailing newline, if
/// any). Interior space/tab runs may be collapsed; trailing whitespace is
/// stripped. Leading indentation is preserved (markdown list nesting and
/// indented-code blocks rely on it).
fn streamline_prose_line(line: &str, cfg: &OutputStreamlineConfig) -> String {
    let (content, nl) = split_newline(line);
    let mut c = content.to_string();
    if cfg.collapse_inline_ws {
        c = collapse_interior_ws(&c);
    }
    if cfg.trim_trailing {
        c = c.trim_end_matches([' ', '\t']).to_string();
    }
    let mut out = c;
    out.push_str(nl);
    out
}

fn split_newline(line: &str) -> (&str, &str) {
    match line.rfind('\n') {
        Some(i) => (&line[..i], &line[i..]),
        None => (line, ""),
    }
}

/// Collapse runs of spaces/tabs to a single space, but only **after** any
/// leading indentation has been copied verbatim.
fn collapse_interior_ws(s: &str) -> String {
    let body_start = s.len() - s.trim_start_matches([' ', '\t']).len();
    let mut out = String::with_capacity(s.len());
    out.push_str(&s[..body_start]);
    let mut in_run = false;
    for ch in s[body_start..].chars() {
        if ch == ' ' || ch == '\t' {
            if !in_run {
                out.push(' ');
                in_run = true;
            }
        } else {
            in_run = false;
            out.push(ch);
        }
    }
    out
}

/// Collapse runs of 2+ consecutive blank *prose* lines into a single blank
/// line. Blank lines inside fenced code (tagged `code`) are preserved and also
/// reset the prose-blank counter so merging never crosses a code boundary.
fn collapse_blank_runs(lines: Vec<LineOut>) -> Vec<LineOut> {
    let mut out: Vec<LineOut> = Vec::with_capacity(lines.len());
    let mut prose_blanks = 0usize;
    for l in lines {
        if l.code {
            prose_blanks = 0;
            out.push(l);
        } else if line_is_blank(&l.text) {
            prose_blanks += 1;
            if prose_blanks <= 1 {
                out.push(l);
            }
        } else {
            prose_blanks = 0;
            out.push(l);
        }
    }
    out
}

/// Drop leading and trailing blank (whitespace-only) lines, keeping the first
/// and last content lines intact — including their indentation.
fn trim_outer_blanks(s: &str) -> String {
    let lines: Vec<&str> = s.split_inclusive('\n').collect();
    let start = lines
        .iter()
        .position(|l| !line_is_blank(l))
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|l| !line_is_blank(l))
        .map(|i| i + 1)
        .unwrap_or(0);
    if start >= end {
        return String::new();
    }
    lines[start..end].concat()
}

fn line_is_blank(line: &str) -> bool {
    line.chars()
        .all(|c| c == ' ' || c == '\t' || c == '\n' || c == '\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> OutputStreamlineConfig {
        OutputStreamlineConfig::default()
    }

    fn off() -> OutputStreamlineConfig {
        OutputStreamlineConfig {
            enabled: false,
            ..Default::default()
        }
    }

    #[test]
    fn clean_text_is_unchanged() {
        let t = "Hello world.\nSecond line.\n";
        assert_eq!(streamline(t, &cfg()), t);
    }

    #[test]
    fn disabled_is_verbatim() {
        let t = "a   \n\n\n\nb   \n";
        assert_eq!(streamline(t, &off()), t);
    }

    #[test]
    fn empty_is_empty() {
        assert_eq!(streamline("", &cfg()), "");
    }

    #[test]
    fn trims_trailing_whitespace_per_prose_line() {
        let t = "keep\t  \n   \ntrim too   \n";
        let got = streamline(t, &cfg());
        assert_eq!(got, "keep\n\ntrim too\n");
    }

    #[test]
    fn collapses_blank_runs_to_one() {
        let t = "a\n\n\n\n\nb\n";
        assert_eq!(streamline(t, &cfg()), "a\n\nb\n");
    }

    #[test]
    fn trims_outer_blank_lines() {
        let t = "\n\n\nhello\n\nworld\n\n\n";
        assert_eq!(streamline(t, &cfg()), "hello\n\nworld\n");
    }

    #[test]
    fn preserves_leading_indentation() {
        // Markdown nested lists / indented code depend on leading spaces.
        let t = "  - item\n    - nested\n";
        assert_eq!(streamline(t, &cfg()), t);
    }

    #[test]
    fn code_fence_preserved_verbatim() {
        // Trailing whitespace and blank lines inside the fence must survive.
        let t = "intro\n```rust\nfn main() {   \n\n\n    let x = 1;   \n}\n```\noutro\n";
        assert_eq!(streamline(t, &cfg()), t);
    }

    #[test]
    fn tildo_fence_preserved_verbatim() {
        let t = "txt\n~~~\n  spaced   \n\n~~~\nend\n";
        assert_eq!(streamline(t, &cfg()), t);
    }

    #[test]
    fn prose_around_fence_is_normalized() {
        let t = "prose a   \n\n\n\n```rust\nx   \n```\n\n\n\nprose b   \n";
        let got = streamline(t, &cfg());
        // Prose loses trailing ws + blank runs collapse; fence interior intact.
        assert_eq!(got, "prose a\n\n```rust\nx   \n```\n\nprose b\n");
    }

    #[test]
    fn collapse_inline_ws_optin_preserves_indent() {
        let on = OutputStreamlineConfig {
            collapse_inline_ws: true,
            ..Default::default()
        };
        // Leading indent kept, interior runs collapsed.
        assert_eq!(
            streamline("    hello    world\t\there", &on),
            "    hello world here"
        );
    }

    #[test]
    fn collapse_inline_ws_off_by_default() {
        assert_eq!(streamline("a    b\t\tc", &cfg()), "a    b\t\tc");
    }

    #[test]
    fn fence_info_string_kept() {
        let t = "```python\nprint('x')   \n```\n";
        assert_eq!(streamline(t, &cfg()), t);
    }

    #[test]
    fn no_trailing_newline_handled() {
        let t = "line one\n\n\n\nline two";
        assert_eq!(streamline(t, &cfg()), "line one\n\nline two");
    }
}
