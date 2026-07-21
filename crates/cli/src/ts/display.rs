//! Pure display helpers for `ts -l` output: relative time bucket, task preview
//! head, home-abbreviated workdir path. All deterministic and unit-tested.

use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn format_ts(created_secs: i64) -> String {
    let delta = (now_secs() - created_secs).max(0);
    if delta < 60 {
        return "just now".into();
    }
    if delta < 3600 {
        return format!("{}m ago", delta / 60);
    }
    if delta < 86400 {
        return format!("{}h ago", delta / 3600);
    }
    format!("{}d ago", delta / 86400)
}

/// First `n` characters of a task string (the `/task` content), with no
/// ellipsis so the count is exact. Used by `ts -l` to show the task head.
pub(crate) fn task_head(s: &str, n: usize) -> String {
    s.trim().chars().take(n).collect()
}

/// Abbreviate an absolute path by replacing the home directory with `~`,
/// keeping the display narrow for the workdir column.
pub(crate) fn abbreviate_path(p: &str) -> String {
    if p.is_empty() {
        return "(?)".into();
    }
    if let Some(home) = std::env::var_os("HOME") {
        if let Some(home_s) = home.to_str() {
            if !home_s.is_empty() && p.starts_with(home_s) {
                return format!("~{}", &p[home_s.len()..]);
            }
        }
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_head_takes_first_n_chars() {
        assert_eq!(task_head("hello world", 10), "hello worl");
        assert_eq!(task_head("short", 10), "short");
        // Leading/trailing whitespace is trimmed before counting.
        assert_eq!(task_head("  abcdefghij  ", 10), "abcdefghij");
    }

    #[test]
    fn abbreviate_path_replaces_home() {
        let home = std::env::var("HOME").unwrap_or_default();
        std::env::set_var("HOME", "/root");
        assert_eq!(abbreviate_path("/root/opencoder"), "~/opencoder");
        assert_eq!(abbreviate_path("/root"), "~");
        assert_eq!(abbreviate_path("/opt/other"), "/opt/other");
        assert_eq!(abbreviate_path(""), "(?)");
        std::env::set_var("HOME", home);
    }

    #[test]
    fn format_ts_relative_buckets() {
        assert_eq!(format_ts(now_secs()), "just now");
        assert_eq!(format_ts(now_secs() - 120), "2m ago");
        assert_eq!(format_ts(now_secs() - 7200), "2h ago");
        assert_eq!(format_ts(now_secs() - 2 * 86400), "2d ago");
    }
}
