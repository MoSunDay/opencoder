//! Core misc tests — ToolFilter::allows + truncate_output.

use opencode_core::{tool::truncate_output, ToolFilter, ToolOutput};

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
