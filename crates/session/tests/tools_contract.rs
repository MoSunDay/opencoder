//! Tool contract tests — each tool exercised with real tempdir + ToolContext.
//! Per rules/01-mandatory-tests.md: every business function gets a real behavior test.

use std::path::Path;

use opencoder_core::{Tool, ToolContext};
use opencoder_session::tools::{
    bash::BashTool, edit::EditTool, glob::GlobTool, grep::GrepTool, ls::ListTool,
    write::WriteTool,
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
        out.content.contains("moved to background"),
        "missing handoff text: {out:?}"
    );
    assert!(
        out.content.contains("/tmp/opencode_bg_"),
        "missing output path: {out:?}"
    );

    // The grandchild should be alive and producing a heartbeat.
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert!(heartbeat.exists(), "grandchild never ran — test setup invalid");
    let s1 = std::fs::metadata(&heartbeat).map(|m| m.len()).unwrap_or(0);
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let s2 = std::fs::metadata(&heartbeat).map(|m| m.len()).unwrap_or(0);
    assert!(
        s2 > s1,
        "grandchild heartbeat stopped ({} -> {} bytes) — process killed prematurely",
        s1, s2
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
        out.content.contains("moved to background"),
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


#[cfg(unix)]
#[tokio::test]
async fn grep_follows_symlink_but_breaks_cycle() {
    // A self-referencing symlink (loop -> .) must not cause infinite recursion.
    // The canonical-path guard deduplicates the real directory, so the match is
    // found exactly once instead of up to the 1000-result cap.
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("real.txt"), "UNIQUE_NEEDLE here").unwrap();
    symlink(".", dir.path().join("loop")).unwrap();
    let c = ctx(dir.path());
    let out = GrepTool
        .execute(json!({"pattern": "UNIQUE_NEEDLE"}), &c)
        .await
        .unwrap();
    let count = out.content.matches("UNIQUE_NEEDLE").count();
    assert_eq!(count, 1, "expected exactly one match (cycle not broken): {out:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn grep_includes_symlinked_directory() {
    // The real directory lives outside the search root and is reachable only
    // through a symlink — proving grep follows symlinked directories.
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("realdir")).unwrap();
    std::fs::write(dir.path().join("realdir").join("deep.txt"), "DEEP_NEEDLE").unwrap();
    std::fs::create_dir(dir.path().join("search")).unwrap();
    symlink("../realdir", dir.path().join("search").join("alias")).unwrap();
    let c = ctx(dir.path());
    let out = GrepTool
        .execute(
            json!({"pattern": "DEEP_NEEDLE", "path": dir.path().join("search").to_str().unwrap()}),
            &c,
        )
        .await
        .unwrap();
    assert!(out.content.contains("DEEP_NEEDLE"), "symlinked dir not searched: {out:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn grep_includes_symlinked_file() {
    // A symlink to a file outside the search root — grep must read through it.
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("realfile.txt"), "FILE_NEEDLE").unwrap();
    std::fs::create_dir(dir.path().join("search")).unwrap();
    symlink("../realfile.txt", dir.path().join("search").join("link.txt")).unwrap();
    let c = ctx(dir.path());
    let out = GrepTool
        .execute(
            json!({"pattern": "FILE_NEEDLE", "path": dir.path().join("search").to_str().unwrap()}),
            &c,
        )
        .await
        .unwrap();
    assert!(out.content.contains("FILE_NEEDLE"), "symlinked file not searched: {out:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn glob_survives_self_referencing_symlink() {
    // Defensive: confirm glob's `**` does not hang on a self-referencing symlink.
    // If glob 0.3.x lacks cycle detection this test would never complete.
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "").unwrap();
    symlink(".", dir.path().join("loop")).unwrap();
    let c = ctx(dir.path());
    let out = GlobTool
        .execute(json!({"pattern": "**/*.rs"}), &c)
        .await
        .unwrap();
    assert!(out.content.contains("a.rs"), "glob result: {out:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn glob_survives_multiple_self_referencing_symlinks() {
    // The real regression point. A single self-loop (`loop -> .`) is a linear
    // chain the kernel cuts off via ELOOP, so it gives a false sense of safety.
    // Two or more self-referencing symlinks in one directory (`a -> .`, `b -> .`)
    // cause branching recursion: 2^depth paths, i.e. a real hang / IO explosion.
    // The canonical-path `seen` set must dedup the real directory so each is
    // visited exactly once. Asserts completion under 5s (would hang without fix).
    use std::os::unix::fs::symlink;
    use std::time::Instant;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("target.rs"), "").unwrap();
    symlink(".", dir.path().join("a")).unwrap();
    symlink(".", dir.path().join("b")).unwrap();
    symlink(".", dir.path().join("c")).unwrap();
    let c = ctx(dir.path());
    let start = Instant::now();
    let out = GlobTool
        .execute(json!({"pattern": "**/*.rs"}), &c)
        .await
        .unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 5,
        "glob took {:?} on triple self-loop; cycle not broken",
        elapsed
    );
    assert!(
        out.content.contains("target.rs"),
        "expected target.rs in results: {out:?}"
    );
    // Result must not blow up toward the 500 cap — the deduped real dir is
    // entered once, so target.rs appears exactly once.
    let count = out.content.matches("target.rs").count();
    assert_eq!(count, 1, "target.rs should appear once, got {count}: {out:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn glob_matches_normal_tree_parity_with_crate() {
    // On a symlink-free mixed tree, the self-written walker (matches_path_with)
    // must produce exactly the same result set as the glob crate's own
    // `glob::glob()` iterator. Guards against matching-semantics drift,
    // including `**`, trailing-`**` (dir-only), and `.hidden` handling.
    use std::path::PathBuf;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("sub/deep")).unwrap();
    std::fs::write(root.join("a.rs"), "").unwrap();
    std::fs::write(root.join("sub/b.rs"), "").unwrap();
    std::fs::write(root.join("sub/deep/c.rs"), "").unwrap();
    std::fs::write(root.join(".hidden.rs"), "").unwrap();
    std::fs::write(root.join("d.txt"), "").unwrap();
    let c = ctx(root);
    for pat in &[
        "**/*.rs",
        "**/*.txt",
        "*.rs",
        "sub/**/*.rs",
        "a.rs",
        "**/.hidden.rs",
        "**/*",
        "sub/**",
    ] {
        let mut crate_results: Vec<String> = glob::glob(&format!("{}/{}", root.display(), pat))
            .unwrap()
            .filter_map(|r| r.ok())
            .map(|p: PathBuf| p.display().to_string())
            .collect();
        crate_results.sort();
        let out = GlobTool
            .execute(json!({"pattern": pat}), &c)
            .await
            .unwrap();
        let mut tool_results: Vec<String> = if out.content == "no matches" {
            Vec::new()
        } else {
            out.content.lines().map(String::from).collect()
        };
        tool_results.sort();
        assert_eq!(
            tool_results, crate_results,
            "parity drift for pattern {:?}:\n  tool : {:?}\n  crate: {:?}",
            pat, tool_results, crate_results
        );
    }
}
