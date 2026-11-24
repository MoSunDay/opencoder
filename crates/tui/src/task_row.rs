//! Row helpers for the `/task` session picker: relative timestamps and
//! preview-line shaping. Pure functions so the picker and its tests can drive
//! them without a terminal.

use crate::composer;
use crate::terminal_text;

/// Timestamp a session entry is shown by: `updated_at` when the store
/// recorded one, otherwise the creation time.
pub fn activity_ts(updated_at: i64, created_at: i64) -> i64 {
    if updated_at > 0 {
        updated_at
    } else {
        created_at
    }
}

/// Human-readable age of `ts_ms` relative to `now_ms`, floored to whole
/// units: "now", "42s ago", "7m ago", "3h ago", "12d ago".
pub fn relative_time(ts_ms: i64, now_ms: i64) -> String {
    const MIN: i64 = 60_000;
    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 86_400_000;
    let diff = now_ms - ts_ms;
    if diff <= 0 {
        return "now".to_string();
    }
    if diff < MIN {
        format!("{}s ago", diff / 1000)
    } else if diff < HOUR {
        format!("{}m ago", diff / MIN)
    } else if diff < DAY {
        format!("{}h ago", diff / HOUR)
    } else {
        format!("{}d ago", diff / DAY)
    }
}

/// Placeholder for a row whose preview and title are both empty. Longer
/// than the 10-column minimum the picker must keep readable.
const EMPTY_TASK_TEXT: &str = "(no task text)";

/// Collapse every whitespace run (including newlines from pasted prompts)
/// into one space so a row always renders as a single line.
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Visible task text for a session row: the stored preview first, then the
/// session title. The title may be a first-user-message digest today and an
/// LLM-generated title later; either way it stays readable even when the
/// preview subquery had to be skipped (unreadable message pages) or the first
/// message carried no visible text. Candidates are sanitized and must have a
/// nonzero display width so control-only/zero-width text cannot yield a blank
/// row.
fn task_text(preview: &str, title: Option<&str>) -> String {
    for candidate in [preview, title.unwrap_or("")] {
        let sanitized = terminal_text::sanitize_single_line(candidate);
        let line = single_line(&sanitized);
        if !line.is_empty() && composer::str_width(&line) > 0 {
            return line;
        }
    }
    EMPTY_TASK_TEXT.to_string()
}

/// Second row of a session entry: the visible task text trimmed to `max_w`
/// display columns. The fallback keeps at least 10 columns worth of text
/// whenever the popup is wide enough.
pub fn preview_line(preview: &str, title: Option<&str>, max_w: usize) -> String {
    composer::truncate_to_width(&task_text(preview, title), max_w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_boundaries() {
        let now = 1_700_000_000_000i64;
        assert_eq!(relative_time(now, now), "now");
        assert_eq!(relative_time(now, now + 59_000), "59s ago");
        assert_eq!(relative_time(now, now + 60_000), "1m ago");
        assert_eq!(relative_time(now, now + 59 * 60_000), "59m ago");
        assert_eq!(relative_time(now, now + 60 * 60_000), "1h ago");
        assert_eq!(relative_time(now, now + 23 * 3_600_000), "23h ago");
        assert_eq!(relative_time(now, now + 24 * 3_600_000), "1d ago");
        assert_eq!(relative_time(now, now + 3 * 86_400_000), "3d ago");
    }

    #[test]
    fn relative_time_future_or_zero_diff_is_now() {
        let now = 1_700_000_000_000i64;
        assert_eq!(relative_time(now + 1, now), "now", "future timestamp");
        assert_eq!(
            relative_time(now - 60_000, now - 60_000),
            "now",
            "zero diff"
        );
    }

    #[test]
    fn activity_ts_prefers_updated_at_and_falls_back() {
        assert_eq!(activity_ts(500, 100), 500);
        assert_eq!(activity_ts(0, 100), 100, "missing updated_at falls back");
        assert_eq!(activity_ts(-1, 100), 100, "sentinel updated_at falls back");
    }

    #[test]
    fn preview_line_empty_preview_falls_back_to_title() {
        assert_eq!(
            preview_line("", Some("specific task text"), 40),
            "specific task text"
        );
        assert_eq!(
            preview_line("   \n\t", Some("specific task text"), 40),
            "specific task text",
            "whitespace-only preview falls through"
        );
    }

    #[test]
    fn preview_line_prefers_preview_when_both_preview_and_title_exist() {
        assert_eq!(
            preview_line("actual prompt", Some("session title"), 40),
            "actual prompt"
        );
    }

    #[test]
    fn preview_line_skips_titles_with_no_visible_width() {
        for title in ["   \n\t", "\u{1b}\u{01}\u{7f}", "\u{200b}\u{200c}"] {
            assert_eq!(
                preview_line("", Some(title), 40),
                "(no task text)",
                "invisible title {title:?} must fall through to the placeholder"
            );
        }
    }

    #[test]
    fn preview_line_without_preview_or_title_keeps_a_readable_placeholder() {
        let out = preview_line("", None, 40);
        assert_eq!(out, "(no task text)");
        assert!(
            composer::str_width(&out) >= 10,
            "placeholder must keep at least 10 columns: {out:?}"
        );
    }

    #[test]
    fn preview_line_collapses_multiline_text_to_one_line() {
        assert_eq!(
            preview_line("first line\nsecond\tline", None, 40),
            "first line second line"
        );
    }

    #[test]
    fn preview_line_truncates_to_width_with_ellipsis() {
        assert_eq!(preview_line("short", None, 20), "short", "fits unchanged");
        let wide = "x".repeat(80);
        let out = preview_line(&wide, None, 10);
        assert_eq!(composer::str_width(&out), 10, "9 cols + ellipsis fits 10");
    }

    #[test]
    fn preview_line_truncates_long_title_to_width_with_ellipsis() {
        let title = "y".repeat(80);
        let out = preview_line("", Some(&title), 12);
        assert_eq!(composer::str_width(&out), 12);
        assert!(out.ends_with('\u{2026}'), "truncation marks the cut: {out}");
    }

    #[test]
    fn preview_line_does_not_tear_wide_chars() {
        // Four CJK glyphs are 8 columns; a 5-column budget must keep one
        // whole glyph plus the ellipsis, never a torn half-width fragment.
        let out = preview_line("\u{4f60}\u{597d}\u{4e16}\u{754c}", None, 5);
        assert_eq!(composer::str_width(&out), 5);
        assert!(out.ends_with('\u{2026}'));
        assert!(out.starts_with('\u{4f60}'), "first glyph kept whole: {out}");
    }

    #[test]
    fn preview_line_does_not_tear_wide_chars_in_title() {
        // Three whole CJK glyphs (6 columns) plus the ellipsis fill 7.
        let out = preview_line("", Some("\u{4f60}\u{597d}\u{4e16}\u{754c}"), 7);
        assert_eq!(out, "\u{4f60}\u{597d}\u{4e16}\u{2026}");
        assert_eq!(composer::str_width(&out), 7);
    }
}
