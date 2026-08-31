//! Row helpers for the `/task` session picker: relative timestamps and
//! preview-line shaping. Pure functions so the picker and its tests can drive
//! them without a terminal.

use crate::composer;

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

/// Second row of a session entry: the session preview trimmed to `max_w`
/// display columns. An empty preview still renders an ellipsis so the row
/// never collapses into a blank line.
pub fn preview_line(preview: &str, max_w: usize) -> String {
    let trimmed = preview.trim();
    if trimmed.is_empty() {
        return "\u{2026}".to_string();
    }
    composer::truncate_to_width(trimmed, max_w)
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
    fn preview_line_empty_falls_back_to_ellipsis() {
        assert_eq!(preview_line("", 20), "\u{2026}");
        assert_eq!(preview_line("   \n\t", 20), "\u{2026}", "whitespace only");
    }

    #[test]
    fn preview_line_truncates_to_width_with_ellipsis() {
        assert_eq!(preview_line("short", 20), "short", "fits unchanged");
        let wide = "x".repeat(80);
        let out = preview_line(&wide, 10);
        assert_eq!(composer::str_width(&out), 10, "9 cols + ellipsis fits 10");
    }

    #[test]
    fn preview_line_does_not_tear_wide_chars() {
        // Four CJK glyphs are 8 columns; a 5-column budget must keep one
        // whole glyph plus the ellipsis, never a torn half-width fragment.
        let out = preview_line("\u{4f60}\u{597d}\u{4e16}\u{754c}", 5);
        assert_eq!(composer::str_width(&out), 5);
        assert!(out.ends_with('\u{2026}'));
        assert!(out.starts_with('\u{4f60}'), "first glyph kept whole: {out}");
    }
}
