use super::*;

#[tokio::test]
async fn bash_normal_completion() {
    let _g = test_registry_mutex().lock().await;
    let tool = BashTool;
    let input = json!({"command": "echo hello; echo world >&2"});
    let out = tool.execute(input, &ctx()).await.unwrap();
    assert!(!out.is_error, "expected success, got: {}", out.content);
    assert!(out.content.contains("hello"), "stdout: {}", out.content);
    assert!(
        out.content.contains("[stderr]"),
        "stderr marker: {}",
        out.content
    );
    assert!(
        out.content.contains("world"),
        "stderr text: {}",
        out.content
    );
    assert!(
        !out.content.contains("[exit code:"),
        "success must not annotate exit code: {}",
        out.content
    );
}

#[tokio::test]
async fn bash_failure_appends_exit_code() {
    let _g = test_registry_mutex().lock().await;
    let tool = BashTool;
    let input = json!({"command": "echo oops; exit 7"});
    let out = tool.execute(input, &ctx()).await.unwrap();
    assert!(
        out.is_error,
        "expected error for non-zero exit: {}",
        out.content
    );
    assert!(out.content.contains("oops"), "stdout: {}", out.content);
    assert!(
        out.content.contains("[exit code: 7]"),
        "failure must annotate exit code: {}",
        out.content
    );
}

#[tokio::test]
#[cfg(unix)]
async fn bash_output_overflow_is_an_error_and_kills_the_group() {
    let _g = test_registry_mutex().lock().await;
    let tool = BashTool;
    let input = json!({
        "command": "timeout 30; dd if=/dev/zero bs=8388609 count=1 2>/dev/null"
    });
    let out = tokio::time::timeout(Duration::from_secs(10), tool.execute(input, &ctx()))
        .await
        .expect("overflow must terminate the command")
        .unwrap();
    assert!(
        out.is_error,
        "overflow must be a tool error: {}",
        out.content
    );
    assert_eq!(
        out.content,
        "output_limit_exceeded: bash stdout exceeds 8388608 bytes"
    );
    assert!(list().is_empty(), "overflowed command must be unregistered");
}

#[tokio::test]
#[cfg(unix)]
async fn background_output_overflow_stops_process_and_caps_file() {
    let _g = test_registry_mutex().lock().await;
    let tool = BashTool;
    // The tool spawns `bash -lc`. Under a minimal environment (systemd
    // transient units, CI) HOME/SHELL are unset: the login shell then
    // resolves the home from /etc/passwd and profile snippets emit
    // stderr into the handoff file, busting the size budget below.
    // Point HOME at an empty directory so the test stays independent
    // of the ambient environment and profile content.
    let home =
        std::env::temp_dir().join(format!("opencoder-bg-overflow-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let bg_ctx = ToolContext {
        extra_env: vec![("HOME".into(), home.to_string_lossy().into_owned())],
        ..ctx()
    };
    // Hide sleep behind a variable so the 1s test foreground timeout is
    // retained. The output overflow happens after handoff.
    let input = json!({
        "command": "d=2; s=sleep; \"$s\" \"$d\"; dd if=/dev/zero bs=8388609 count=1 2>/dev/null; \"$s\" 30"
    });
    let out = tool.execute(input, &bg_ctx).await.unwrap();
    assert!(
        !out.is_error,
        "handoff itself remains successful: {}",
        out.content
    );
    let pid: u32 = out
        .content
        .lines()
        .find_map(|line| line.strip_prefix("pid: "))
        .expect("handoff pid")
        .parse()
        .unwrap();
    let path = output_path(pid);
    tokio::time::timeout(Duration::from_secs(10), async {
        while list().iter().any(|info| info.pid == pid) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("overflow must terminate the background process");

    let bytes = std::fs::read(&path).unwrap();
    let marker = b"output_limit_exceeded: bash stdout exceeds 8388608 bytes";
    assert!(
        bytes.windows(marker.len()).any(|window| window == marker),
        "background file must report the overflow"
    );
    assert!(
        bytes.len() < crate::tools::bg::STREAM_OUTPUT_LIMIT_BYTES + 256,
        "background file must stop at the stream limit"
    );
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir_all(&home);
}

/// A short command (completes well under the test timeout of 1 s) returns
/// its own output without triggering the timeout/handoff path.
#[tokio::test]
async fn bash_short_command_completes_normally() {
    let _g = test_registry_mutex().lock().await;
    let tool = BashTool;
    let input = json!({"command": "echo done"});
    let out = tool.execute(input, &ctx()).await.unwrap();
    assert!(!out.is_error, "expected success, got: {}", out.content);
    assert!(
        out.content.contains("done"),
        "expected the command's own output, got: {}",
        out.content
    );
    assert!(
        !out.content.contains(BASH_TIMEOUT_MARKER),
        "short command must not trigger timeout handoff: {}",
        out.content
    );
}

/// A command that backgrounds a grandchild (e.g. `cmd &`) which inherits
/// the stdout pipe returns promptly on natural completion: the group kill
/// reaps the leaked grandchild so the drain tasks reach EOF instead of
/// hanging forever (bash has no runner deadline, so nothing else breaks it).
#[tokio::test]
#[cfg(unix)]
async fn bash_returns_when_grandchild_leaks_pipe() {
    let _g = test_registry_mutex().lock().await;
    let tool = BashTool;
    // `sleep 30 &` spawns a process that inherits stdout; bash exits at
    // once (wait returns Ok) but the grandchild keeps the pipe open.
    let input = json!({"command": "echo done; sleep 30 &"});
    let result = tokio::time::timeout(Duration::from_secs(10), tool.execute(input, &ctx())).await;
    assert!(
        result.is_ok(),
        "bash should return within 10s even when a grandchild holds the pipe"
    );
    let out = result.unwrap().unwrap();
    assert!(
        out.content.contains("done"),
        "expected the command's output, got: {}",
        out.content
    );
    kill_all();
}
