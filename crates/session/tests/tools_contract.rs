//! Tool contract tests — each tool exercised with real tempdir + ToolContext.
//! Per rules/01-mandatory-tests.md: every business function gets a real behavior test.

use std::path::Path;

use opencoder_core::{Tool, ToolContext};
use opencoder_session::tools::{
    bash::BashTool, edit::EditTool, glob::GlobTool, ls::ListTool, write::WriteTool,
};
use serde_json::json;

fn ctx(dir: &Path) -> ToolContext {
    ToolContext {
        session_id: "test-session".into(),
        message_id: "test-msg".into(),
        agent: "act".into(),
        working_dir: dir.to_path_buf(),
        max_output: 4096,
        proxy: None,
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
        .execute(
            json!({"path": "sub/dir/file.rs", "content": "fn main() {}"}),
            &c,
        )
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
        .execute(
            json!({"path": "f.txt", "old_string": "foo", "new_string": "bar"}),
            &c,
        )
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
    assert_eq!(
        std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
        "bar bar bar"
    );
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
    let out = ListTool.execute(json!({}), &c).await.unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("file1.txt"));
    assert!(out.content.contains("subdir/"));
}

#[tokio::test]
#[cfg(unix)]
async fn bash_tool_captures_stdout_via_pipe() {
    // Per rules/01-mandatory-tests.md: the captured-pipe contract for the bash
    // tool. Output must come back through ToolOutput, not leak to the terminal.
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let out = BashTool
        .execute(json!({"command": "echo hello-from-bash"}), &c)
        .await
        .unwrap();
    assert!(!out.is_error, "unexpected error: {out:?}");
    assert!(
        out.content.contains("hello-from-bash"),
        "stdout missing: {out:?}"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn bash_tool_captures_stderr_via_pipe() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let out = BashTool
        .execute(json!({"command": "echo oops 1>&2"}), &c)
        .await
        .unwrap();
    assert!(out.content.contains("oops"), "stderr missing: {out:?}");
    assert!(
        out.content.contains("[stderr]"),
        "stderr marker missing: {out:?}"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn bash_tool_detaches_controlling_terminal() {
    // Regression for "bash output lands in the input area": the child must run
    // in its own session (setsid) so it cannot write to /dev/tty and corrupt the
    // TUI composer. Signal: the bash process is a session leader, i.e. its
    // session id (sid) equals its pid. Without setsid the sid would be the
    // parent (test runner) session and the two would differ.
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let out = BashTool
        .execute(
            json!({"command": "ps -o pid=,sid= -p \"$$\" | tr -s ' '"}),
            &c,
        )
        .await
        .unwrap();
    let nums: Vec<u64> = out
        .content
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    assert_eq!(nums.len(), 2, "expected 'pid sid', got: {out:?}");
    assert_eq!(
        nums[0], nums[1],
        "child is NOT a session leader — setsid() not applied: {out:?}"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn bash_tool_kills_process_group_on_timeout() {
    // Regression: on timeout the bash tool must kill the *entire* child process
    // group, not only the direct bash child. Otherwise grandchildren (builds,
    // servers, test runners, backgrounded jobs) survive as orphans. We spawn a
    // grandchild that beats a heartbeat file, time the tool out, then verify the
    // heartbeat stops growing — proving the grandchild died with the group.
    //
    // Non-interactive `bash -lc` keeps job control OFF, so the backgrounded
    // pipeline stays in bash's process group → `kill(-pgid, SIGKILL)` reaches it.
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let heartbeat = dir.path().join("heartbeat");
    let pidfile = dir.path().join("gpid");
    let command = format!(
        "sh -c 'echo $$ > {pid}; while true; do echo x >> {hb}; sleep 0.2; done' & sleep 30",
        pid = pidfile.display(),
        hb = heartbeat.display(),
    );

    let out = BashTool
        .execute(json!({"command": command, "timeout": 1}), &c)
        .await
        .unwrap();
    assert!(out.is_error);
    assert!(out.content.contains("timed out"), "unexpected: {out:?}");

    // The grandchild should have produced a heartbeat during the 1s window.
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert!(
        heartbeat.exists(),
        "grandchild never ran — test setup invalid"
    );

    // Sample the heartbeat twice; if the grandchild is dead the file is static.
    let s1 = std::fs::metadata(&heartbeat).map(|m| m.len()).unwrap_or(0);
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let s2 = std::fs::metadata(&heartbeat).map(|m| m.len()).unwrap_or(0);
    assert_eq!(
        s1, s2,
        "grandchild kept writing ({} -> {} bytes): process-group kill failed",
        s1, s2
    );

    // Cleanup: if a buggy build left the grandchild alive, kill it so the test
    // never leaks a runaway process.
    if let Ok(txt) = std::fs::read_to_string(&pidfile) {
        if let Ok(pid) = txt.trim().parse::<i32>() {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

#[tokio::test]
#[cfg(unix)]
async fn bash_tool_returns_partial_output_on_timeout() {
    // On timeout the tool must surface whatever the command printed before it
    // hung — that output is usually the only clue to *why* it hung (the failing
    // test, the blocking syscall, the last build step). Discarding it (the old
    // behavior) forces the agent to blindly retry. We print a unique marker to
    // stdout, then block forever; after a 1s timeout the partial marker must be
    // present in the returned (error) output.
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let out = BashTool
        .execute(
            json!({"command": "echo PARTIAL-MARKER-9f3a; sleep 30", "timeout": 1}),
            &c,
        )
        .await
        .unwrap();
    assert!(out.is_error, "expected error on timeout: {out:?}");
    assert!(
        out.content.contains("timed out"),
        "missing timeout banner: {out:?}"
    );
    assert!(
        out.content.contains("PARTIAL-MARKER-9f3a"),
        "partial output discarded (should be surfaced): {out:?}"
    );
}
