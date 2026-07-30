//! Tool contract tests — each tool exercised with real tempdir + ToolContext.
//! Per rules/01-mandatory-tests.md: every business function gets a real behavior test.

use std::path::Path;

use opencoder_core::{Tool, ToolContext};
use opencoder_session::tools::{
    bash::BashTool, edit::EditTool, ls::ListTool, search::SearchTool,
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
async fn bash_tool_hands_off_on_timeout() {
    // On timeout the bash tool must NOT kill the command — instead it hands
    // off to a background supervisor and returns a guidance message with the
    // PID and output file path. The process stays alive.
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let heartbeat = dir.path().join("heartbeat");
    let pidfile = dir.path().join("gpid");
    let command = format!(
        "sh -c 'echo $$ > {pid}; while true; do echo x >> {hb}; sleep 0.2; done' & sleep 4",
        pid = pidfile.display(),
        hb = heartbeat.display(),
    );

    let out = BashTool
        .execute(json!({"command": command, "timeout": 1}), &c)
        .await
        .unwrap();
    // Handoff is not an error.
    assert!(!out.is_error, "expected ok, got: {out:?}");
    assert!(
        out.content.contains("Moved to background"),
        "missing handoff text: {out:?}"
    );
    assert!(
        out.content.contains("/tmp/opencode_bg_"),
        "missing output path: {out:?}"
    );

    // The grandchild should be alive and producing a heartbeat.
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert!(
        heartbeat.exists(),
        "grandchild never ran — test setup invalid"
    );
    let s1 = std::fs::metadata(&heartbeat).map(|m| m.len()).unwrap_or(0);
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let s2 = std::fs::metadata(&heartbeat).map(|m| m.len()).unwrap_or(0);
    assert!(
        s2 > s1,
        "grandchild heartbeat stopped ({} -> {} bytes) — process killed prematurely",
        s1,
        s2
    );

    // Extract pid and wait for the output file to get [exit code:].
    let pid_str = out
        .content
        .split("PID: ")
        .nth(1)
        .and_then(|s| s.split('.').next())
        .unwrap_or("");
    let pid: u32 = pid_str.parse().unwrap_or(0);
    assert!(pid > 0, "could not parse pid: {out:?}");
    let path = std::path::PathBuf::from(format!("/tmp/opencode_bg_{pid}.output"));

    // Wait for the command (sleep 4) to exit and the supervisor to clean up.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    let mut got_exit_code = false;
    while std::time::Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if content.contains("[exit code:") {
                got_exit_code = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(got_exit_code, "output file never got [exit code:]");

    // After exit + supervisor kill, the heartbeat should be static.
    let s3 = std::fs::metadata(&heartbeat).map(|m| m.len()).unwrap_or(0);
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let s4 = std::fs::metadata(&heartbeat).map(|m| m.len()).unwrap_or(0);
    assert_eq!(
        s3, s4,
        "heartbeat kept growing ({} -> {}) — supervisor failed to kill group",
        s3, s4
    );

    // Cleanup.
    let _ = std::fs::remove_file(&path);
    if let Ok(txt) = std::fs::read_to_string(&pidfile) {
        if let Ok(gpid) = txt.trim().parse::<i32>() {
            unsafe { libc::kill(gpid, libc::SIGKILL) };
        }
    }
}

#[tokio::test]
#[cfg(unix)]
async fn bash_tool_output_file_captures_output_on_timeout() {
    // On timeout, the command's output is streamed to the background output
    // file. We print a unique marker, block briefly, and verify the file
    // contains the marker and the exit code.
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let out = BashTool
        .execute(
            json!({"command": "echo PARTIAL-MARKER-9f3a; sleep 3", "timeout": 1}),
            &c,
        )
        .await
        .unwrap();
    assert!(!out.is_error, "expected ok on handoff: {out:?}");
    assert!(
        out.content.contains("Moved to background"),
        "missing handoff text: {out:?}"
    );

    let pid_str = out
        .content
        .split("PID: ")
        .nth(1)
        .and_then(|s| s.split('.').next())
        .unwrap_or("");
    let pid: u32 = pid_str.parse().unwrap_or(0);
    assert!(pid > 0, "could not parse pid: {out:?}");
    let path = std::path::PathBuf::from(format!("/tmp/opencode_bg_{pid}.output"));

    // Wait for the command to exit and the supervisor to append [exit code:].
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    let mut got = false;
    while std::time::Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if content.contains("PARTIAL-MARKER-9f3a") && content.contains("[exit code: 0]") {
                got = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(
        got,
        "output file missing marker or exit code: {:?}",
        std::fs::read_to_string(&path).ok()
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn search_finds_matching_lines() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn alpha() {}\nfn beta() {}").unwrap();
    std::fs::write(dir.path().join("b.txt"), "beta beta").unwrap();
    let c = ctx(dir.path());
    let out = SearchTool
        .execute(json!({"pattern": "beta"}), &c)
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    // Both files contain "beta"; output is `relpath:line: content`.
    assert!(out.content.contains("a.rs:2: fn beta() {}"), "{}", out.content);
    assert!(out.content.contains("b.txt:1: beta beta"), "{}", out.content);
    // Non-matching content must not appear.
    assert!(!out.content.contains("alpha"));
}

#[tokio::test]
async fn search_returns_no_matches_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn alpha() {}").unwrap();
    let c = ctx(dir.path());
    let out = SearchTool
        .execute(json!({"pattern": "zzz_nomatch"}), &c)
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert_eq!(out.content, "no matches");
}

#[tokio::test]
async fn search_include_filter_restricts_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "NEEDLE").unwrap();
    std::fs::write(dir.path().join("b.txt"), "NEEDLE").unwrap();
    let c = ctx(dir.path());
    let out = SearchTool
        .execute(json!({"pattern": "NEEDLE", "include": "*.rs"}), &c)
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("a.rs"), "{}", out.content);
    assert!(!out.content.contains("b.txt"), "{}", out.content);
}

#[tokio::test]
async fn search_searches_subdirectories() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("sub/deep")).unwrap();
    std::fs::write(dir.path().join("sub/deep/c.rs"), "DEEP_NEEDLE").unwrap();
    let c = ctx(dir.path());
    let out = SearchTool
        .execute(json!({"pattern": "DEEP_NEEDLE"}), &c)
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("c.rs"), "{}", out.content);
    assert!(out.content.contains("DEEP_NEEDLE"), "{}", out.content);
}

#[tokio::test]
async fn search_invalid_regex_errors() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let out = SearchTool
        .execute(json!({"pattern": "*"}), &c)
        .await
        .unwrap();
    assert!(out.is_error, "invalid regex must error: {out:?}");
    assert!(out.content.contains("invalid regex"), "{}", out.content);
}

#[tokio::test]
async fn search_single_file_target() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn foo() {}\nfn bar() {}").unwrap();
    std::fs::write(dir.path().join("b.rs"), "fn bar() {}").unwrap();
    let c = ctx(dir.path());
    let out = SearchTool
        .execute(
            json!({"pattern": "bar", "path": "a.rs"}),
            &c,
        )
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("a.rs:2: fn bar()"), "{}", out.content);
    // b.rs must not be searched: only the single file was targeted.
    assert!(!out.content.contains("b.rs"), "{}", out.content);
}

#[tokio::test]
async fn search_regex_anchors_work() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn begin\nbegin middle\nend begin").unwrap();
    let c = ctx(dir.path());
    let out = SearchTool
        .execute(json!({"pattern": "^fn"}), &c)
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    // Only the line starting with "fn".
    assert!(out.content.contains("a.rs:1: fn begin"), "{}", out.content);
    let count = out.content.matches("a.rs:").count();
    assert_eq!(count, 1, "anchored pattern matches once: {out:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn search_follows_symlinked_file() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("real.txt"), "LINK_NEEDLE").unwrap();
    std::fs::create_dir(dir.path().join("search")).unwrap();
    symlink("../real.txt", dir.path().join("search").join("link.txt")).unwrap();
    let c = ctx(dir.path());
    let out = SearchTool
        .execute(
            json!({"pattern": "LINK_NEEDLE", "path": dir.path().join("search").to_str().unwrap()}),
            &c,
        )
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("LINK_NEEDLE"), "symlinked file not searched: {out:?}");
}
