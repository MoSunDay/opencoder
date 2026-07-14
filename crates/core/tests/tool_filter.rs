//! Core misc tests — ToolFilter::allows + truncate_output.

use opencoder_core::{
    tool::{truncate_output, truncate_output_with_error},
    ToolFilter, ToolOutput,
};

#[test]
fn tool_filter_all_allows_everything() {
    let filter = ToolFilter::All;
    assert!(filter.allows("bash"));
    assert!(filter.allows("edit"));
    assert!(filter.allows("anything"));
}

#[test]
fn tool_filter_allow_list_gates_tools() {
    let filter = ToolFilter::Allow(vec!["read".into(), "glob".into(), "grep".into()]);
    assert!(filter.allows("read"));
    assert!(filter.allows("glob"));
    assert!(filter.allows("grep"));
    assert!(
        !filter.allows("bash"),
        "bash should be blocked by plan-agent filter"
    );
    assert!(!filter.allows("edit"), "edit should be blocked");
    assert!(!filter.allows("write"), "write should be blocked");
}

#[test]
fn truncate_output_short_content_passes_through() {
    let out = truncate_output("hello".into(), 100);
    assert!(!out.is_error);
    assert_eq!(out.content, "hello");
}

#[test]
fn truncate_output_long_content_gets_preview() {
    let long = "x".repeat(5000);
    let out = truncate_output(long.clone(), 1000);
    assert!(!out.is_error);
    assert!(out.content.contains("truncated"));
    assert!(out.content.len() < long.len());
}

#[test]
fn tool_output_ok_and_err_constructors() {
    let ok = ToolOutput::ok("success");
    assert!(!ok.is_error);
    assert_eq!(ok.content, "success");

    let err = ToolOutput::err("failure");
    assert!(err.is_error);
    assert_eq!(err.content, "failure");
}

#[test]
fn truncate_output_with_error_preserves_error_flag() {
    let long = "x".repeat(10_000);
    let out = truncate_output_with_error(long, 100, true);
    assert!(
        out.is_error,
        "is_error must be preserved when truncating error output"
    );
    assert!(
        out.content.contains("truncated"),
        "truncation marker must be present"
    );
}

#[test]
fn truncate_output_with_error_preserves_ok_flag() {
    let long = "x".repeat(10_000);
    let out = truncate_output_with_error(long, 100, false);
    assert!(
        !out.is_error,
        "is_error must remain false for non-error output"
    );
}

#[test]
fn truncate_output_with_error_no_truncation_when_under_max() {
    let out = truncate_output_with_error("short".to_string(), 100, true);
    assert!(out.is_error);
    assert_eq!(out.content, "short");
}

#[test]
fn truncate_output_line_limit_truncates() {
    let content: String = (0..1000).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let out = truncate_output(content.clone(), 100_000);
    assert!(!out.is_error);
    assert!(
        out.content.contains("[output truncated"),
        "should contain truncation marker"
    );
    assert!(
        out.content.contains("1000 lines"),
        "should report original line count"
    );
    // Resulting line count should be at most 801 (800 lines + truncation notice line)
    assert!(
        out.content.lines().count() <= 802,
        "truncated output should have at most ~800 lines + notice"
    );
}

#[test]
fn truncate_output_byte_limit_truncates() {
    let content = "x".repeat(10_000);
    let out = truncate_output(content.clone(), 100_000);
    assert!(
        out.content.contains("[output truncated"),
        "should contain truncation marker"
    );
    assert!(
        out.content.contains("10000 bytes"),
        "should report original byte count"
    );
}

#[test]
fn truncate_output_both_limits_exceeded() {
    let content: String = (0..2000)
        .map(|i| format!("line {i} with some padding text here"))
        .collect::<Vec<_>>()
        .join("\n");
    let out = truncate_output_with_error(content.clone(), 100_000, false);
    assert!(
        out.content.contains("2000 lines"),
        "should report original line count"
    );
    assert!(
        out.content.contains("bytes"),
        "should also report byte count since content is large"
    );
}

#[test]
fn truncate_output_under_both_limits_passes_through() {
    let content = "hello\nworld\nthis is short";
    let out = truncate_output(content.to_string(), 4096);
    assert_eq!(out.content, content);
}

#[test]
fn truncate_output_does_not_split_multibyte_chars() {
    // Each of these is a 3-byte UTF-8 char; create content well over 4096 bytes.
    let content = "世".repeat(2000); // 6000 bytes
    let out = truncate_output(content, 4096);
    assert!(out.content.contains("truncated"));
    // The truncated prefix should still be valid UTF-8 (no panic means valid).
    assert!(out.content.starts_with("世"));
}
