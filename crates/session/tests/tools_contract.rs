//! Tool contract tests — each tool exercised with real tempdir + ToolContext.
//! Per rules/01-mandatory-tests.md: every business function gets a real behavior test.

use std::path::Path;

use opencode_core::{Tool, ToolContext};
use opencode_session::tools::{edit::EditTool, glob::GlobTool, ls::ListTool, plan::PlanExitTool, write::WriteTool};
use serde_json::json;

fn ctx(dir: &Path) -> ToolContext {
    ToolContext {
        session_id: "test-session".into(),
        message_id: "test-msg".into(),
        agent: "act".into(),
        working_dir: dir.to_path_buf(),
        max_output: 4096,
    }
}

#[tokio::test]
async fn write_tool_creates_file_with_content() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let out = WriteTool
        .execute(json!({"path": "hello.txt", "content": "line1\nline2"}), &c)
        .await
        .unwrap();
    assert!(!out.is_error);
    let written = std::fs::read_to_string(dir.path().join("hello.txt")).unwrap();
    assert_eq!(written, "line1\nline2");
}

#[tokio::test]
async fn write_tool_creates_parent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let out = WriteTool
        .execute(json!({"path": "sub/dir/file.rs", "content": "fn main() {}"}), &c)
        .await
        .unwrap();
    assert!(!out.is_error);
    assert!(dir.path().join("sub/dir/file.rs").exists());
}

#[tokio::test]
async fn edit_tool_replaces_exact_string() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("code.rs");
    std::fs::write(&path, "fn old_name() {}").unwrap();
    let c = ctx(dir.path());
    let out = EditTool
        .execute(
            json!({"path": "code.rs", "old_string": "old_name", "new_string": "new_name"}),
            &c,
        )
        .await
        .unwrap();
    assert!(!out.is_error);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn new_name() {}");
}

#[tokio::test]
async fn edit_tool_errors_on_not_found() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
    let c = ctx(dir.path());
    let out = EditTool
        .execute(
            json!({"path": "f.txt", "old_string": "nonexistent", "new_string": "x"}),
            &c,
        )
        .await
        .unwrap();
    assert!(out.is_error);
    assert!(out.content.contains("not found"));
}

#[tokio::test]
async fn edit_tool_errors_on_ambiguous_without_replace_all() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f.txt"), "foo foo foo").unwrap();
    let c = ctx(dir.path());
    let out = EditTool
        .execute(json!({"path": "f.txt", "old_string": "foo", "new_string": "bar"}), &c)
        .await
        .unwrap();
    assert!(out.is_error);
    assert!(out.content.contains("3 times"));
}

#[tokio::test]
async fn edit_tool_replace_all() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f.txt"), "foo foo foo").unwrap();
    let c = ctx(dir.path());
    let out = EditTool
        .execute(
            json!({"path": "f.txt", "old_string": "foo", "new_string": "bar", "replace_all": true}),
            &c,
        )
        .await
        .unwrap();
    assert!(!out.is_error);
    assert_eq!(std::fs::read_to_string(dir.path().join("f.txt")).unwrap(), "bar bar bar");
}

#[tokio::test]
async fn glob_tool_matches_pattern() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "").unwrap();
    std::fs::write(dir.path().join("b.rs"), "").unwrap();
    std::fs::write(dir.path().join("c.txt"), "").unwrap();
    let c = ctx(dir.path());
    let out = GlobTool
        .execute(json!({"pattern": "*.rs"}), &c)
        .await
        .unwrap();
    assert!(!out.is_error);
    assert!(out.content.contains("a.rs"));
    assert!(out.content.contains("b.rs"));
    assert!(!out.content.contains("c.txt"));
}

#[tokio::test]
async fn ls_tool_lists_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file1.txt"), "").unwrap();
    std::fs::create_dir(dir.path().join("subdir")).unwrap();
    let c = ctx(dir.path());
    // No path → defaults to working_dir
    let out = ListTool
        .execute(json!({}), &c)
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("file1.txt"));
    assert!(out.content.contains("subdir/"));
}

#[tokio::test]
async fn plan_exit_writes_plan_file() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let out = PlanExitTool
        .execute(json!({"plan": "# My Plan\n- Step 1", "filename": "sprint-1"}), &c)
        .await
        .unwrap();
    assert!(!out.is_error);
    let plan_path = dir.path().join(".opencode/plans/sprint-1.md");
    assert!(plan_path.exists());
    let content = std::fs::read_to_string(&plan_path).unwrap();
    assert!(content.contains("# My Plan"));
    assert!(content.contains("Step 1"));
}
