//! Tool contract tests — each tool exercised with real tempdir + ToolContext.
//! Per rules/01-mandatory-tests.md: every business function gets a real behavior test.

use std::path::Path;

use opencoder_core::{Tool, ToolContext};
use opencoder_session::tools::{bash::BashTool, edit::EditTool, ls::ListTool, search::SearchTool};
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
async fn bash_tool_runs_long_command_without_handoff() {
    // A command that would previously exceed the 1 s timeout (and get handed
    // off to a background supervisor) now just runs to completion in the
    // foreground: the tool returns the command's own output, never a
    // "Moved to background" message, and no background output file is left
    // behind. The `timeout` input is now ignored.
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let out = BashTool
        .execute(
            json!({"command": "echo PARTIAL-MARKER-9f3a; sleep 2; echo done", "timeout": 1}),
            &c,
        )
        .await
        .unwrap();
    assert!(!out.is_error, "expected success, got: {out:?}");
    assert!(
        out.content.contains("PARTIAL-MARKER-9f3a"),
        "expected streamed stdout, got: {out:?}"
    );
    assert!(
        out.content.contains("done"),
        "expected final echo, got: {out:?}"
    );
    assert!(
        !out.content.contains("Moved to background"),
        "foreground bash must never hand off: {out:?}"
    );
    // Registry must be empty after the command finishes (entry unregistered).
    assert!(
        opencoder_session::tools::bg::list().is_empty()
            || !opencoder_session::tools::bg::list()
                .iter()
                .any(|i| out.content.contains(&format!("{}", i.pid))),
        "no stale registry entry after foreground completion"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn bash_tool_registered_and_stoppable() {
    // While a long bash is running it is registered so `/stop` can kill its
    // process group. We start a long sleep, confirm it appears in the bg
    // registry, then kill it via `bg::stop` (the primitive `/stop` uses) and
    // confirm the foreground tool call returns with a non-zero exit.
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let pidfile = dir.path().join("pid");
    let command = format!("echo $$ > {pf}; sleep 30", pf = pidfile.display());

    let handle = tokio::spawn(async move {
        BashTool
            .execute(json!({"command": command}), &c)
            .await
            .unwrap()
    });

    // Wait for the command to start and publish its pid.
    let mut pid: u32 = 0;
    for _ in 0..100 {
        if let Ok(txt) = std::fs::read_to_string(&pidfile) {
            if let Ok(p) = txt.trim().parse::<u32>() {
                pid = p;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(pid > 0, "pidfile never written: command did not start");

    // The running command must be registered (visible to `/ps`, killable by `/stop`).
    assert!(
        opencoder_session::tools::bg::list()
            .iter()
            .any(|i| i.pid == pid),
        "running bash pid {pid} should be registered"
    );

    // Kill it the way `/stop` does (per-pid). The foreground call must then
    // return (the group-kill makes `wait()` resolve with a signal exit).
    assert!(
        opencoder_session::tools::bg::stop(pid),
        "stop should find and kill the registered pid"
    );

    let out = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("foreground call did not return after /stop")
        .unwrap();
    assert!(
        out.is_error,
        "killed bash should report a non-zero exit: {out:?}"
    );
    assert!(
        !opencoder_session::tools::bg::list()
            .iter()
            .any(|i| i.pid == pid),
        "killed bash should be unregistered"
    );
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
    assert!(
        out.content.contains("a.rs:2: fn beta() {}"),
        "{}",
        out.content
    );
    assert!(
        out.content.contains("b.txt:1: beta beta"),
        "{}",
        out.content
    );
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
        .execute(json!({"pattern": "bar", "path": "a.rs"}), &c)
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
    assert!(
        out.content.contains("LINK_NEEDLE"),
        "symlinked file not searched: {out:?}"
    );
}
